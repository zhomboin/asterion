import { useEffect, useRef, useState, type ClipboardEvent, type KeyboardEvent } from 'react';
import { ApiError, errorMessage, isUnauthorized, request } from '../api';
import { Screen, initialTerminalSize } from './Screen';

const keys: Record<string, string> = {
  Enter: '\r', Tab: '\t', Escape: '\x1b', Backspace: '\x7f', Delete: '\x1b[3~',
  ArrowUp: '\x1b[A', ArrowDown: '\x1b[B', ArrowRight: '\x1b[C', ArrowLeft: '\x1b[D',
  Home: '\x1b[H', End: '\x1b[F', Insert: '\x1b[2~', PageUp: '\x1b[5~', PageDown: '\x1b[6~',
  F1: '\x1bOP', F2: '\x1bOQ', F3: '\x1bOR', F4: '\x1bOS', F5: '\x1b[15~',
  F6: '\x1b[17~', F7: '\x1b[18~', F8: '\x1b[19~', F9: '\x1b[20~',
  F10: '\x1b[21~', F11: '\x1b[23~', F12: '\x1b[24~',
};

type Output = { data: string; offset: number; closed: boolean; truncated: boolean };
const encoder = new TextEncoder();
const inputLimit = 4096;

function encode(text: string) {
  return btoa(String.fromCharCode(...encoder.encode(text)));
}

function decode(text: string) {
  return Uint8Array.from(atob(text), (character) => character.charCodeAt(0));
}

export function TerminalSession({ id, onExpired, onClose }: {
  id: string;
  onExpired: () => void;
  onClose: () => void;
}) {
  const path = '/terminal/' + encodeURIComponent(id);
  const [terminal] = useState(() => new Screen(initialTerminalSize.cols, initialTerminalSize.rows));
  const [screen, setScreen] = useState(() => terminal.snapshot());
  const [closed, setClosed] = useState(false);
  const [ending, setEnding] = useState(false);
  const [truncated, setTruncated] = useState(false);
  const [inputError, setInputError] = useState('');
  const [outputError, setOutputError] = useState('');
  const [paste, setPaste] = useState('');
  const view = useRef<HTMLPreElement>(null);
  const followOutput = useRef(true);
  const input = useRef<HTMLTextAreaElement>(null);
  const probe = useRef<HTMLSpanElement>(null);
  const queue = useRef('');
  const running = useRef(false);
  const composing = useRef(false);
  const active = useRef<AbortController | null>(null);
  const releaseTimer = useRef<number | undefined>(undefined);

  useEffect(() => {
    const controller = new AbortController();
    let decoder = new TextDecoder();
    let delay: number | undefined;
    let offset = 0;
    window.clearTimeout(releaseTimer.current);
    active.current = controller;
    input.current?.focus();

    async function read() {
      try {
        const output = await request<Output>(path + '?offset=' + offset, { signal: controller.signal });
        if (controller.signal.aborted) return;
        const changed = output.data !== '' || output.offset !== offset;
        offset = output.offset;
        if (output.truncated) {
          decoder = new TextDecoder();
          terminal.reset();
          setTruncated(true);
        }
        if (changed || output.truncated || output.closed) {
          terminal.write(decoder.decode(decode(output.data), { stream: !output.closed }));
          setScreen(terminal.snapshot());
        }
        setOutputError('');
        if (output.closed) {
          setClosed(true);
          return;
        }
        delay = window.setTimeout(read, 400);
      } catch (failure) {
        if (controller.signal.aborted) return;
        if (isUnauthorized(failure)) {
          onExpired();
          return;
        }
        if (failure instanceof ApiError && (failure.status === 404 || failure.status === 410)) {
          setClosed(true);
          return;
        }
        setOutputError(errorMessage(failure));
        delay = window.setTimeout(read, 2_000);
      }
    }

    function release() {
      void request<unknown>(path, { method: 'DELETE', keepalive: true }).catch(() => undefined);
    }

    void read();
    window.addEventListener('pagehide', release);
    return () => {
      controller.abort();
      window.clearTimeout(delay);
      window.removeEventListener('pagehide', release);
      releaseTimer.current = window.setTimeout(release, 0);
    };
  }, [path, onExpired, terminal]);

  useEffect(() => {
    if (!view.current || !probe.current || closed || ending) return;
    const box = view.current;
    let timeout: number | undefined;
    let last = initialTerminalSize;
    let pending: typeof initialTerminalSize | undefined;
    let sending = false;
    let stopped = false;

    async function applySize() {
      const signal = active.current?.signal;
      if (!signal || sending) return;
      sending = true;
      try {
        while (!stopped && pending && !signal.aborted) {
          const size = pending;
          pending = undefined;
          if (size.cols === last.cols && size.rows === last.rows) continue;
          await request(path + '/resize', { method: 'POST', body: size, signal, response: 'empty' });
          if (signal.aborted || stopped) return;
          last = size;
          terminal.resize(size.cols, size.rows);
          setScreen(terminal.snapshot());
        }
      } catch (failure) {
        if (!signal.aborted && !stopped) {
          if (isUnauthorized(failure)) onExpired();
          else setInputError(errorMessage(failure));
        }
      } finally {
        sending = false;
      }
    }

    const observer = new ResizeObserver(() => {
      window.clearTimeout(timeout);
      timeout = window.setTimeout(() => {
        const character = probe.current?.getBoundingClientRect();
        if (!character?.width || !character.height) return;
        const cols = Math.max(20, Math.min(400, Math.floor((box.clientWidth - 28) / character.width)));
        const rows = Math.max(5, Math.min(200, Math.floor((box.clientHeight - 24) / character.height)));
        pending = { cols, rows };
        void applySize();
      }, 140);
    });
    observer.observe(box);
    return () => {
      stopped = true;
      observer.disconnect();
      window.clearTimeout(timeout);
    };
  }, [closed, ending, path, onExpired, terminal]);

  useEffect(() => {
    if (view.current && followOutput.current) view.current.scrollTop = view.current.scrollHeight;
  }, [screen]);

  async function send(text: string) {
    if (text === '' || closed || ending) return;
    if (encoder.encode(queue.current + text).length > inputLimit) {
      setInputError('终端单次输入过长。');
      return;
    }
    queue.current += text;
    if (running.current) return;

    const signal = active.current?.signal;
    running.current = true;
    setInputError('');
    try {
      while (queue.current && !signal?.aborted) {
        const pending = queue.current;
        queue.current = '';
        await request<unknown>(path + '/input', { method: 'POST', body: { data: encode(pending) }, signal });
      }
    } catch (failure) {
      queue.current = '';
      if (!signal?.aborted) {
        if (isUnauthorized(failure)) onExpired();
        else setInputError(errorMessage(failure));
      }
    } finally {
      running.current = false;
    }
  }

  function keydown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.nativeEvent.isComposing || event.metaKey || event.getModifierState('AltGraph')) return;
    let command = keys[event.key];
    if (event.ctrlKey && !event.altKey && !event.shiftKey && /^[a-z@[\\\]^_]$/iu.test(event.key) && event.key.toLowerCase() !== 'v') {
      command = String.fromCharCode(event.key.toUpperCase().charCodeAt(0) & 31);
    } else if (event.altKey && !event.ctrlKey && event.key.length === 1) command = '\x1b' + event.key;
    if (!command || (event.key === 'Tab' && event.shiftKey)) return;
    event.preventDefault();
    void send(command);
  }

  function pasteInput(event: ClipboardEvent<HTMLTextAreaElement>) {
    event.preventDefault();
    const text = event.clipboardData.getData('text/plain');
    if (encoder.encode(text).length > inputLimit - 20) {
      setInputError('终端单次粘贴过长。');
    } else if (/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/u.test(text)) {
      setInputError('粘贴内容含控制字符，未发送。');
    } else if (/[\r\n]/u.test(text)) {
      setPaste(text);
    } else {
      sendPaste(text);
    }
  }

  function sendPaste(text: string) {
    const value = text.replace(/\r\n|\r|\n/gu, '\r');
    void send(terminal.bracketedPaste ? '\x1b[200~' + value + '\x1b[201~' : value);
    setPaste('');
  }

  async function disconnect() {
    if (closed) {
      onClose();
      return;
    }
    queue.current = '';
    setEnding(true);
    try {
      await request<unknown>(path, { method: 'DELETE' });
      setClosed(true);
      onClose();
    } catch (failure) {
      if (isUnauthorized(failure)) onExpired();
      else setInputError(errorMessage(failure));
    } finally {
      setEnding(false);
    }
  }

  return (
    <div className="terminal-session">
      <div className="section-bar">
        <h2>终端会话</h2>
        <button type="button" onClick={() => void disconnect()} disabled={ending}>{closed ? '关闭' : '断开'}</button>
      </div>
      {outputError && <p role="alert" className="error">{outputError}</p>}
      {inputError && <p role="alert" className="error">{inputError}</p>}
      {truncated && <p className="observation">旧输出已截断</p>}
      <div className="terminal-display">
        <span ref={probe} aria-hidden="true" className="terminal-probe">M</span>
        <pre ref={view} className="terminal-screen" tabIndex={0} aria-label="终端屏幕" onScroll={(event) => {
          const screen = event.currentTarget;
          followOutput.current = screen.scrollHeight - screen.scrollTop - screen.clientHeight <= 24;
        }}>
          {screen.lines.map((line, index) => (
            <span key={index} className="terminal-row">
              {!closed && screen.cursor?.row === index
                ? <>{screen.cursor.before}<span className="terminal-cursor">{screen.cursor.character}</span>{screen.cursor.after}</>
                : line || ' '}
            </span>
          ))}
        </pre>
      </div>
      {closed ? <p className="observation" role="status">会话已结束</p> : (
        <label className="terminal-input">
          终端输入
          <textarea
            ref={input}
            rows={1}
            autoComplete="off"
            disabled={ending}
            spellCheck={false}
            onKeyDown={keydown}
            onPaste={pasteInput}
            onCompositionStart={() => { composing.current = true; }}
            onCompositionEnd={(event) => {
              composing.current = false;
              void send(event.currentTarget.value);
              event.currentTarget.value = '';
            }}
            onChange={(event) => {
              if (composing.current) return;
              void send(event.currentTarget.value);
              event.currentTarget.value = '';
            }}
          />
        </label>
      )}
      {paste && !closed && (
        <div className="detail-panel terminal-paste">
          <p>粘贴的多行文本可能立即执行。</p>
          <pre className="output">{paste}</pre>
          <div className="dialog-actions">
            <button type="button" onClick={() => setPaste('')}>取消</button>
            <button type="button" className="primary" onClick={() => sendPaste(paste)}>确认发送</button>
          </div>
        </div>
      )}
    </div>
  );
}
