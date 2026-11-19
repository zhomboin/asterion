import { useEffect, useRef } from 'react';
import { deploymentTranscript } from './deploymentTranscript';

export function DeploymentLog({ lines, target }: { lines: string[]; target?: string }) {
  const output = useRef<HTMLPreElement>(null);
  const follow = useRef(true);
  useEffect(() => {
    if (output.current && follow.current) output.current.scrollTop = output.current.scrollHeight;
  }, [lines.length]);

  return (
    <pre className="output deployment-output" role="log" aria-label="SSH 部署日志" ref={output} tabIndex={0}
      onScroll={(event) => {
        const element = event.currentTarget;
        follow.current = element.scrollHeight - element.scrollTop - element.clientHeight < 32;
      }}>
      {lines.length ? deploymentTranscript(lines, target) : '正在等待部署日志…'}
    </pre>
  );
}
