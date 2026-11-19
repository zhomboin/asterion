import { useEffect, useRef } from 'react';
import { groupDeploymentLog } from './groupDeploymentLog';

const labels = { command: '执行命令', output: '日志输出', error: '错误' };

export function DeploymentLog({ lines, failed }: { lines: string[]; failed: boolean }) {
  const output = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  useEffect(() => {
    if (output.current && follow.current) output.current.scrollTop = output.current.scrollHeight;
  }, [lines.length]);

  return (
    <div className="deployment-output deployment-log" role="log" aria-label="SSH 部署日志" ref={output} tabIndex={0}
      onScroll={(event) => {
        const element = event.currentTarget;
        follow.current = element.scrollHeight - element.scrollTop - element.clientHeight < 32;
      }}>
      {lines.length === 0 && <p className="muted">正在等待部署日志…</p>}
      {groupDeploymentLog(lines, failed).map((block, index) => (
        <div className="deployment-log-block" data-kind={block.kind} key={index}>
          <strong className="deployment-log-label">{labels[block.kind]}</strong>
          <div>{block.lines.map((line, i) => <pre key={i}>{line}</pre>)}</div>
        </div>
      ))}
    </div>
  );
}
