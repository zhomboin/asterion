import { useEffect, useState } from 'react';
import type { Node } from '../api';
import { NodeStatus, PageHeader, Problem, Tabs, Timestamp } from '../ui';
import { useResource } from '../useResource';
import { CollectorDeployment } from './CollectorDeployment';
import { MetricsPanel } from './MetricsPanel';
import { ProcessesPanel } from './ProcessesPanel';
import { DockerPanel } from './DockerPanel';
import { SystemdPanel } from './SystemdPanel';
import { CommandPanel } from '../operations/CommandPanel';
import { ProposalDialog } from '../operations/ProposalDialog';
import type { NodeOperations, OperationAction, OperationCatalog } from '../operations/data';
import { TerminalPanel } from '../terminal/TerminalPanel';
import { AccountsPanel } from '../accounts/AccountsPanel';
import { ManagedPanel } from '../operations/ManagedPanel';
import { FilePanel } from '../operations/FilePanel';

const tabs = [
  ['metrics', '指标'],
  ['collector', '监控部署'],
  ['processes', '进程'],
  ['docker', 'Docker'],
  ['systemd', 'systemd'],
  ['accounts', 'Linux 账号'],
] as const;

type Tab = (typeof tabs)[number][0] | 'command' | 'terminal' | 'packages' | 'configs' | 'files';

export function NodeDetailPage({ id, onExpired }: { id: string; onExpired: () => void }) {
  const base = '/nodes/' + encodeURIComponent(id);
  const node = useResource<Node>(base, onExpired, 5_000);
  const [initialUnit] = useState(() => new URLSearchParams(window.location.search).get('service') ?? undefined);
  const [tab, setTab] = useState<Tab>(initialUnit ? 'systemd' : 'metrics');
  const [fileDirty, setFileDirty] = useState(false);
  const data = node.data;
  const online = data?.status === 'online' && !node.error;
  const operationCatalog = useResource<OperationCatalog>(online ? base + '/operation-catalog' : null, onExpired, 30_000);
  const terminal = useResource<{ enabled: boolean }>(online ? base + '/terminal/capability' : null, onExpired, 30_000);
  const hasTerminal = Boolean(online && terminal.data?.enabled && !terminal.error);
  const [proposal, setProposal] = useState<{ action: OperationAction; name?: string; content?: string }>();
  const operations: NodeOperations | undefined = online && operationCatalog.data?.enabled && !operationCatalog.error
    ? { catalog: operationCatalog.data, propose: (action, name, content) => setProposal({ action, name, content }) }
    : undefined;
  const canOperate = operations !== undefined;
  useEffect(() => {
    if (!canOperate) setProposal(undefined);
  }, [canOperate]);

  const hasCommands = Boolean(operations?.catalog.commands.length);
  const activeTab = (tab === 'command' && !hasCommands) || (tab === 'terminal' && !hasTerminal) ? 'metrics' : tab;

  const tabItems: (readonly [Tab, string])[] = [
    ...tabs,
    ['packages', '软件安装'], ['files', '文件编辑'], ['configs', '服务配置'],
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
              <a className="text-link" href={'/operations?node_id=' + encodeURIComponent(id)}>任务记录</a>
              <a className="text-link" href={base + '/manage'}>SSH 连接</a>
              <NodeStatus node={data} />
            </>}
          />
          <dl className="detail-facts node-facts">
            <div><dt>系统</dt><dd>{[data.os, data.arch].filter(Boolean).join(' · ') || '—'}</dd></div>
            <div><dt>SSH 连接</dt><dd>{data.ssh_profile ?? '—'}</dd></div>
            <div><dt>最近连接</dt><dd><Timestamp value={data.last_seen_at} /></dd></div>
            <div><dt>节点 ID</dt><dd className="technical">{data.id}</dd></div>
          </dl>
          {data.ssh_error && <p role="alert">{data.ssh_error}</p>}
          <Tabs label="节点监控视图" items={tabItems} value={activeTab} onChange={(next) => {
            if (next === activeTab || (fileDirty && !window.confirm('放弃当前未保存的修改？'))) return;
            setFileDirty(false);
            setTab(next);
          }} />
          {activeTab === 'collector' && <CollectorDeployment key={id} node={data} onExpired={onExpired} />}
          {activeTab === 'metrics' && <MetricsPanel base={base} onExpired={onExpired} />}
          {activeTab === 'files' && <FilePanel key={id} base={base} node={data} online={online}
            catalog={operationCatalog} onExpired={onExpired} dirty={fileDirty} setDirty={setFileDirty} />}
          {activeTab === 'accounts' && !online && (
            <p className="empty">{node.error ? '暂时无法确认 SSH 状态' : data.status === 'pending' ? 'SSH 尚未连接' : 'SSH 离线，实时查询暂不可用'}</p>
          )}
          {activeTab === 'processes' && <ProcessesPanel base={base} online={online} onExpired={onExpired} operations={operations} />}
          {activeTab === 'docker' && <DockerPanel base={base} online={online} onExpired={onExpired} operations={operations} />}
          {(activeTab === 'packages' || activeTab === 'configs' || activeTab === 'docker') &&
            <ManagedPanel key={id + activeTab} group={activeTab === 'docker' ? 'containers' : activeTab}
              base={base} nodeId={id} online={online} controls={operations} onExpired={onExpired} />}
          {activeTab === 'systemd' && <SystemdPanel
            base={base}
            connectionState={node.error ? 'unknown' : data.status}
            initialUnit={initialUnit}
            onExpired={onExpired}
            operations={operations}
          />}
          {activeTab === 'command' && operations && <CommandPanel controls={operations} />}
          {activeTab === 'terminal' && hasTerminal && <TerminalPanel nodeId={id} onExpired={onExpired} />}
          {online && activeTab === 'accounts' && <AccountsPanel base={base} onExpired={onExpired} operations={operations} />}
          {proposal && operations && (
            <ProposalDialog
              nodeId={id}
              nodeName={data.name}
              action={proposal.action}
              resourceName={proposal.name}
              content={proposal.content}
              onClose={() => setProposal(undefined)}
              onExpired={onExpired}
            />
          )}
        </>
      )}
    </>
  );
}
