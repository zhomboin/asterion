import { useEffect, useState } from 'react';
import type { Node } from '../api';
import { NodeStatus, PageHeader, Problem, Tabs, Timestamp } from '../ui';
import { useResource } from '../useResource';
import { MetricsPanel } from './MetricsPanel';
import { ProcessesPanel } from './ProcessesPanel';
import { DockerPanel } from './DockerPanel';
import { SystemdPanel } from './SystemdPanel';
import { CommandPanel } from '../operations/CommandPanel';
import { ProposalDialog } from '../operations/ProposalDialog';
import type { NodeOperations, OperationAction, OperationCatalog } from '../operations/data';
import { TerminalPanel } from '../terminal/TerminalPanel';
import { AccountsPanel } from '../accounts/AccountsPanel';

const tabs = [
  ['metrics', '指标'],
  ['processes', '进程'],
  ['docker', 'Docker'],
  ['systemd', 'systemd'],
  ['accounts', 'Linux 账号'],
] as const;

type Tab = (typeof tabs)[number][0] | 'command' | 'terminal';

export function NodeDetailPage({ id, onExpired }: { id: string; onExpired: () => void }) {
  const base = '/nodes/' + encodeURIComponent(id);
  const node = useResource<Node>(base, onExpired, 5_000);
  const [tab, setTab] = useState<Tab>('metrics');
  const data = node.data;
  const online = data?.status === 'online' && !node.error;
  const operationCatalog = useResource<OperationCatalog>(online ? base + '/operation-catalog' : null, onExpired, 30_000);
  const terminal = useResource<{ enabled: boolean }>(online ? base + '/terminal/capability' : null, onExpired, 30_000);
  const hasTerminal = Boolean(terminal.data?.enabled && !terminal.error);
  const [proposal, setProposal] = useState<{ action: OperationAction; name?: string }>();
  const operations: NodeOperations | undefined = operationCatalog.data?.enabled && !operationCatalog.error
    ? { catalog: operationCatalog.data, propose: (action, name) => setProposal({ action, name }) }
    : undefined;
  const canOperate = operations !== undefined;
  useEffect(() => {
    if (!canOperate) setProposal(undefined);
  }, [canOperate]);

  const hasCommands = Boolean(operations?.catalog.commands.length);
  const activeTab = (tab === 'command' && !hasCommands) || (tab === 'terminal' && !hasTerminal) ? 'metrics' : tab;

  const tabItems: (readonly [Tab, string])[] = [
    ...tabs,
    ...(hasCommands ? [['command', '预定义命令'] as const] : []),
    ...(hasTerminal ? [['terminal', '终端'] as const] : []),
  ];

  return (
    <>
      {!data && <PageHeader back={{ href: '/nodes', label: '节点列表' }} title="节点详情" />}
      {node.loading && <p role="status">正在加载节点…</p>}
      {node.error && <Problem error={node.error} retry={node.refresh} />}
      {data && (
        <>
          <PageHeader
            back={{ href: '/nodes', label: '节点列表' }}
            title={data.name}
            scope={<>{data.kind === 'wsl' ? '监控范围 · WSL' : '监控范围 · Linux'}{data.wsl_distro && ' · ' + data.wsl_distro}</>}
            actions={<>
              {operations && <a className="text-link" href={'/operations?node_id=' + encodeURIComponent(id)}>任务记录</a>}
              <NodeStatus node={data} />
            </>}
          />
          <dl className="detail-facts node-facts">
            <div><dt>系统</dt><dd>{[data.os, data.arch].filter(Boolean).join(' · ') || '—'}</dd></div>
            <div><dt>Agent 版本</dt><dd>{data.agent_version ?? '—'}</dd></div>
            <div><dt>最近心跳</dt><dd><Timestamp value={data.last_seen_at} /></dd></div>
            <div><dt>节点 ID</dt><dd className="technical">{data.id}</dd></div>
          </dl>
          <Tabs label="节点监控视图" items={tabItems} value={activeTab} onChange={setTab} />
          {activeTab === 'metrics' && <MetricsPanel base={base} onExpired={onExpired} />}
          {activeTab !== 'metrics' && data.status !== 'online' && (
            <p className="empty">{data.status === 'pending' ? 'Agent 尚未连接' : 'Agent 离线，实时查询暂不可用'}</p>
          )}
          {data.status === 'online' && activeTab === 'processes' && <ProcessesPanel base={base} onExpired={onExpired} operations={operations} />}
          {data.status === 'online' && activeTab === 'docker' && <DockerPanel base={base} onExpired={onExpired} operations={operations} />}
          {data.status === 'online' && activeTab === 'systemd' && <SystemdPanel base={base} onExpired={onExpired} operations={operations} />}
          {activeTab === 'command' && operations && <CommandPanel controls={operations} />}
          {activeTab === 'terminal' && hasTerminal && <TerminalPanel nodeId={id} onExpired={onExpired} />}
          {data.status === 'online' && activeTab === 'accounts' && <AccountsPanel base={base} onExpired={onExpired} operations={operations} />}
          {proposal && operations && (
            <ProposalDialog
              nodeId={id}
              nodeName={data.name}
              action={proposal.action}
              resourceName={proposal.name}
              onClose={() => setProposal(undefined)}
              onExpired={onExpired}
            />
          )}
        </>
      )}
    </>
  );
}
