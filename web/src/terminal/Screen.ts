type Mode = 'text' | 'escape' | 'csi' | 'string' | 'string_escape' | 'charset';
type Point = { row: number; column: number };

type Buffer = {
  lines: string[][];
  point: Point;
  saved: Point;
  top: number;
  bottom: number;
};

export const initialTerminalSize = { cols: 80, rows: 24 };
const historySize = 400;
const space = ' ';

export class Screen {
  private mode: Mode = 'text';
  private csi = '';
  private buffer: Buffer;
  private original: Buffer | null = null;
  private history: string[] = [];
  private cursorVisible = true;
  bracketedPaste = false;

  private columns: number;
  private rows: number;

  constructor(columns: number, rows: number) {
    this.columns = columns;
    this.rows = rows;
    this.buffer = this.emptyBuffer();
  }

  reset() {
    this.buffer = this.emptyBuffer();
    this.original = null;
    this.history = [];
    this.mode = 'text';
    this.csi = '';
    this.cursorVisible = true;
    this.bracketedPaste = false;
  }

  resize(columns: number, rows: number) {
    const { buffer } = this;
    this.columns = columns;
    this.rows = rows;
    buffer.lines = this.fit(buffer.lines);
    buffer.point.row = Math.min(rows - 1, buffer.point.row);
    buffer.point.column = Math.min(columns - 1, buffer.point.column);
    buffer.saved.row = Math.min(rows - 1, buffer.saved.row);
    buffer.saved.column = Math.min(columns - 1, buffer.saved.column);
    buffer.top = 0;
    buffer.bottom = rows - 1;
    if (this.original) {
      this.original.lines = this.fit(this.original.lines);
      this.original.point.row = Math.min(rows - 1, this.original.point.row);
      this.original.point.column = Math.min(columns - 1, this.original.point.column);
      this.original.saved.row = Math.min(rows - 1, this.original.saved.row);
      this.original.saved.column = Math.min(columns - 1, this.original.saved.column);
      this.original.top = 0;
      this.original.bottom = rows - 1;
    }
  }

  write(text: string) {
    for (const character of text) {
      switch (this.mode) {
        case 'text': this.text(character); break;
        case 'escape': this.escape(character); break;
        case 'csi':
          if (character >= '@' && character <= '~') {
            this.control(character);
            this.mode = 'text';
          } else if (character === '\x1b') {
            this.mode = 'escape';
          } else if (this.csi.length <= 96) {
            this.csi += character;
          }
          break;
        case 'string':
          if (character === '\x07' || character === '\u009c') this.mode = 'text';
          else if (character === '\x1b') this.mode = 'string_escape';
          break;
        case 'string_escape':
          this.mode = character === '\\' || character === '\x07' ? 'text' : 'string';
          break;
        case 'charset': this.mode = 'text'; break;
      }
    }
  }

  snapshot() {
    const { point, lines } = this.buffer;
    const history = this.original ? [] : this.history;
    return {
      lines: [...history, ...lines.map((line) => line.join('').trimEnd())],
      cursor: this.cursorVisible ? {
        row: history.length + point.row,
        before: lines[point.row].slice(0, Math.min(point.column, this.columns - 1)).join(''),
        character: lines[point.row][Math.min(point.column, this.columns - 1)] || space,
        after: lines[point.row].slice(Math.min(point.column + 1, this.columns)).join('').trimEnd(),
      } : undefined,
    };
  }

  private line() {
    return Array<string>(this.columns).fill(space);
  }

  private fit(lines: string[][]) {
    return Array.from({ length: this.rows }, (_, index) => lines[index]
      ? [...lines[index], ...this.line()].slice(0, this.columns)
      : this.line());
  }

  private emptyBuffer(): Buffer {
    return {
      lines: this.fit([]),
      point: { row: 0, column: 0 },
      saved: { row: 0, column: 0 },
      top: 0,
      bottom: this.rows - 1,
    };
  }

  private text(character: string) {
    const { point, lines } = this.buffer;
    if (character === '\x1b') {
      this.mode = 'escape';
      return;
    }
    if (character === '\u009b') {
      this.csi = '';
      this.mode = 'csi';
      return;
    }
    if ('\u0090\u0098\u009d\u009e\u009f'.includes(character)) {
      this.mode = 'string';
      return;
    }
    if (character === '\r') point.column = 0;
    else if (character === '\n' || character === '\v' || character === '\f') this.scrollDown();
    else if (character === '\b') point.column = Math.max(0, point.column - 1);
    else if (character === '\t') point.column = Math.min(this.columns - 1, (Math.floor(point.column / 8) + 1) * 8);
    else if (character >= space && !(character >= '\x7f' && character <= '\u009f')) {
      if (/^\p{Mark}$/u.test(character) && point.column > 0) {
        const line = lines[point.row];
        const previous = Math.max(0, point.column - (line[point.column - 1] === '' ? 2 : 1));
        if (line[previous].length + character.length <= 32) line[previous] += character;
        return;
      }
      const width = cellWidth(character);
      if (point.column + width > this.columns) {
        point.column = 0;
        this.scrollDown();
      }
      lines[point.row][point.column] = character;
      if (width === 2) lines[point.row][point.column + 1] = '';
      point.column += width;
    }
  }

  private escape(character: string) {
    const { point } = this.buffer;
    this.mode = 'text';
    if (character === '[') {
      this.csi = '';
      this.mode = 'csi';
    } else if (']P^_X'.includes(character)) {
      this.mode = 'string';
    } else if ('()*+#'.includes(character)) {
      this.mode = 'charset';
    } else if (character === 'c') {
      this.reset();
    } else if (character === 'D' || character === 'E') {
      if (character === 'E') point.column = 0;
      this.scrollDown();
    } else if (character === 'M') {
      if (point.row === this.buffer.top) {
        this.buffer.lines.splice(this.buffer.bottom, 1);
        this.buffer.lines.splice(this.buffer.top, 0, this.line());
      } else point.row = Math.max(0, point.row - 1);
    } else if (character === '7') {
      this.buffer.saved = { ...point };
    } else if (character === '8') {
      this.buffer.point = { ...this.buffer.saved };
    }
  }

  private scrollDown() {
    const { point, top, bottom, lines } = this.buffer;
    if (point.row === bottom) {
      const [removed] = lines.splice(top, 1);
      lines.splice(bottom, 0, this.line());
      if (!this.original && top === 0 && bottom === this.rows - 1) {
        this.history.push(removed.join('').trimEnd());
        if (this.history.length > historySize) this.history.shift();
      }
    } else {
      point.row = Math.min(this.rows - 1, point.row + 1);
    }
  }

  private control(final: string) {
    if (!/^\??[\d;]*$/u.test(this.csi)) return;
    const privateMode = this.csi.startsWith('?');
    const args = this.csi.replace(/^\?/u, '').split(';').map(Number);
    if (privateMode) {
      if (final === 'h' || final === 'l') this.updateMode(args, final === 'h');
      return;
    }
    const value = args[0] || 1;
    const { point, lines } = this.buffer;
    const move = (row: number, column: number) => {
      point.row = Math.min(this.rows - 1, Math.max(0, row));
      point.column = Math.min(this.columns - 1, Math.max(0, column));
    };
    switch (final) {
      case 'A': move(point.row - value, point.column); break;
      case 'B': move(point.row + value, point.column); break;
      case 'C': move(point.row, point.column + value); break;
      case 'D': move(point.row, point.column - value); break;
      case 'E': move(point.row + value, 0); break;
      case 'F': move(point.row - value, 0); break;
      case 'G': case '`': move(point.row, value - 1); break;
      case 'd': move(value - 1, point.column); break;
      case 'H': case 'f': move(value - 1, (args[1] || 1) - 1); break;
      case 's': this.buffer.saved = { ...point }; break;
      case 'u': move(this.buffer.saved.row, this.buffer.saved.column); break;
      case 'J': this.eraseDisplay(args[0]); break;
      case 'K':
        if (args[0] === 1) lines[point.row].fill(space, 0, point.column + 1);
        else if (args[0] === 2) lines[point.row].fill(space);
        else lines[point.row].fill(space, point.column);
        break;
      case 'X': lines[point.row].fill(space, point.column, point.column + value); break;
      case 'P': {
        const column = Math.min(point.column, this.columns - 1);
        const removed = lines[point.row].splice(column, Math.min(this.columns - column, value)).length;
        lines[point.row].push(...this.line().slice(0, removed));
        break;
      }
      case '@': {
        const column = Math.min(point.column, this.columns - 1);
        lines[point.row].splice(column, 0, ...this.line().slice(0, Math.min(this.columns, value)));
        lines[point.row].length = this.columns;
        break;
      }
      case 'r':
        this.buffer.top = Math.min(this.rows - 1, Math.max(0, value - 1));
        this.buffer.bottom = Math.min(this.rows - 1, Math.max(this.buffer.top, (args[1] || this.rows) - 1));
        move(0, 0);
        break;
    }
  }

  private eraseDisplay(mode: number) {
    const { point, lines } = this.buffer;
    if (mode === 3) {
      this.history = [];
    } else if (mode === 2) {
      for (const line of lines) line.fill(space);
    } else if (mode === 1) {
      for (const line of lines.slice(0, point.row)) line.fill(space);
      lines[point.row].fill(space, 0, point.column + 1);
    } else if (mode === 0) {
      lines[point.row].fill(space, point.column);
      for (const line of lines.slice(point.row + 1)) line.fill(space);
    }
  }

  private updateMode(args: number[], enabled: boolean) {
    if (args.includes(25)) this.cursorVisible = enabled;
    if (args.includes(2004)) this.bracketedPaste = enabled;
    if (args.some((number) => number === 1049 || number === 1047 || number === 47)) {
      if (enabled && !this.original) {
        this.original = this.buffer;
        this.buffer = this.emptyBuffer();
      } else if (!enabled && this.original) {
        this.buffer = this.original;
        this.original = null;
      }
    }
  }
}

function cellWidth(character: string) {
  const point = character.codePointAt(0) ?? 0;
  return (point >= 0x1100 && point <= 0x115f)
    || (point >= 0x2e80 && point <= 0xa4cf)
    || (point >= 0xac00 && point <= 0xd7a3)
    || (point >= 0xf900 && point <= 0xfaff)
    || (point >= 0xfe10 && point <= 0xfe6f)
    || (point >= 0xff00 && point <= 0xff60)
    || (point >= 0x1f300 && point <= 0x1faff)
    || (point >= 0x20000 && point <= 0x3ffff) ? 2 : 1;
}
