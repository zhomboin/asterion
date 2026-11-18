import { readFileSync } from 'node:fs';
import { Agent } from 'node:https';
import { getCACertificates } from 'node:tls';
import react from '@vitejs/plugin-react';
import { defineConfig, type ProxyOptions } from 'vite';

const base = {
  plugins: [react()],
  build: { assetsDir: 'static' },
};

function requiredFile(name: string) {
  const path = process.env[name];
  if (!path) throw new Error('请配置 ' + name);
  return readFileSync(path);
}

export default defineConfig(({ command, isPreview }) => {
  if (command !== 'serve' || isPreview) return base;

  const target = new URL(process.env.ASTERION_DEV_SERVER_URL ?? 'https://127.0.0.1:8080');
  if (target.protocol !== 'https:' || target.username || target.password
    || target.pathname !== '/' || target.search || target.hash) {
    throw new Error('ASTERION_DEV_SERVER_URL 必须是中心的 HTTPS 源地址');
  }

  const caFile = process.env.ASTERION_DEV_CA_FILE;
  const proxy: ProxyOptions = {
    target: target.origin,
    secure: true,
    agent: caFile ? new Agent({ ca: [...getCACertificates('default'), readFileSync(caFile)] }) : undefined,
  };

  return {
    ...base,
    server: {
      host: '127.0.0.1',
      strictPort: true,
      cors: false,
      https: {
        cert: requiredFile('ASTERION_TLS_CERT_FILE'),
        key: requiredFile('ASTERION_TLS_KEY_FILE'),
      },
      proxy: {
        '/healthz': proxy,
        '/api/v1/agent/connect': { ...proxy, ws: true },
        '/api/v1': proxy,
      },
    },
  };
});
