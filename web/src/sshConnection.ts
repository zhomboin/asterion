export function parseDestination(profile?: string | null) {
  const address = profile?.startsWith('ssh://') ? profile.slice(6) : '';
  const parts = /^(?:([^@]+)@)?(?:\[([^\]]+)\]|([^:]+))(?::(\d+))?$/u.exec(address);
  return { user: parts?.[1] ?? '', host: parts?.[2] ?? parts?.[3] ?? '', port: parts?.[4] ?? '' };
}

export function destination(host: string, user: string, port: string) {
  const hostname = host.trim().replace(/^\[([^\]]+)\]$/u, '$1');
  const address = hostname.includes(':') ? '[' + hostname + ']' : hostname;
  return 'ssh://' + (user.trim() ? user.trim() + '@' : '') + address + (port ? ':' + port : '');
}

export function readConnection(data: FormData) {
  const value = (name: string) => String(data.get(name) ?? '');
  const method = value('ssh_method');
  const ssh_profile = destination(value('ssh_host'), value('ssh_user'), value('ssh_port'));
  if (method === 'key') return { ssh_profile, ssh_auth: { method, identity_file: value('identity_file').trim() } };
  if (method === 'password') return { ssh_profile, ssh_auth: { method, password: value('ssh_password') || undefined } };
  return { ssh_profile, ssh_auth: { method: 'default' } };
}
