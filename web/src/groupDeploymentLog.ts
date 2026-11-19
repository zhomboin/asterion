type LogBlock = { kind: 'command' | 'output' | 'error'; lines: string[] };

export function groupDeploymentLog(records: readonly string[], failed: boolean): LogBlock[] {
  const blocks: LogBlock[] = [];
  for (const [index, record] of records.entries()) {
    if (failed && index === records.length - 1) {
      blocks.push({ kind: 'error', lines: [record] });
    } else if (record.startsWith('$ ')) {
      // 部署器为每条命令单独发送一条带前缀的记录，多行脚本也在同一条内。
      blocks.push({ kind: 'command', lines: [record.slice(2)] });
    } else {
      const previous = blocks.at(-1);
      if (previous?.kind === 'output') previous.lines.push(record);
      else blocks.push({ kind: 'output', lines: [record] });
    }
  }
  return blocks;
}
