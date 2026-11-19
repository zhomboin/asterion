export function deploymentTranscript(records: readonly string[], target = 'ssh'): string {
  // 一条部署记录可以包含整个多行脚本，只替换记录开头的命令标记。
  return records.map(record => record.startsWith('$ ') ? `${target}:~# ${record.slice(2)}` : record).join('\n');
}
