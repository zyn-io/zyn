import { defineConfig, loadEnv } from 'vite';

export const DAEMON_ROLES = new Set(['creator', 'a', 'b']);
export const READ_ROUTES = new Set([
  '/api/overview',
  '/api/offers',
  '/api/anchors',
  '/api/deposit-address',
]);
export const WRITE_ROUTES = new Set([
  '/api/fund',
  '/api/mint',
  '/api/offer',
  '/api/take',
  '/api/cancel-offer',
  '/api/redeem',
  '/api/force',
  '/api/bind',
  '/api/withdraw',
]);

export function resolveBridgeRoute(method, requestUrl) {
  const parsed = new URL(requestUrl, 'http://localhost');
  const [, role, api, name, ...rest] = parsed.pathname.split('/');
  if (!DAEMON_ROLES.has(role) || api !== 'api' || rest.length) return null;
  const path = `/api/${name}`;
  const allowed = method === 'GET' ? READ_ROUTES.has(path) : method === 'POST' && WRITE_ROUTES.has(path);
  return allowed ? { role, path, search: parsed.search } : null;
}

async function readJsonBody(req) {
  const chunks = [];
  let size = 0;
  for await (const chunk of req) {
    size += chunk.length;
    if (size > 64 * 1024) throw new Error('Request body is too large.');
    chunks.push(chunk);
  }
  const text = Buffer.concat(chunks).toString('utf8');
  if (!text) return undefined;
  JSON.parse(text);
  return text;
}

function daemonBridge(targets) {
  function setup(server) {
    server.middlewares.use('/daemon', async (req, res) => {
      res.setHeader('Content-Type', 'application/json; charset=utf-8');
      res.setHeader('Cache-Control', 'no-store');
      const route = resolveBridgeRoute(req.method, req.url);
      if (!route) {
        res.statusCode = 405;
        return res.end(JSON.stringify({ error: 'This daemon route is not allowlisted.' }));
      }
      const target = targets[route.role];
      if (!target) {
        res.statusCode = 503;
        return res.end(JSON.stringify({ error: `No daemon URL is configured for ${route.role}.` }));
      }
      try {
        const body = req.method === 'POST' ? await readJsonBody(req) : undefined;
        const response = await fetch(new URL(route.path + route.search, target), {
          method: req.method,
          headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
          body,
          signal: AbortSignal.timeout(15000),
          redirect: 'error',
        });
        const text = await response.text();
        res.statusCode = response.status;
        res.end(text || JSON.stringify({ ok: response.ok }));
      } catch (error) {
        res.statusCode = error?.message === 'Request body is too large.' ? 413 : 502;
        res.end(JSON.stringify({ error: error?.message === 'Request body is too large.' ? error.message : `The configured ${route.role} daemon could not be reached.` }));
      }
    });
  }
  return { name: 'explicit-three-wallet-daemon-bridge', configureServer: setup, configurePreviewServer: setup };
}

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), 'ZYN_');
  const targets = {
    creator: process.env.ZYN_DAEMON_CREATOR || env.ZYN_DAEMON_CREATOR || 'http://127.0.0.1:8977',
    a: process.env.ZYN_DAEMON_A || env.ZYN_DAEMON_A || 'http://127.0.0.1:8978',
    b: process.env.ZYN_DAEMON_B || env.ZYN_DAEMON_B || 'http://127.0.0.1:8979',
  };
  for (const [role, target] of Object.entries(targets)) {
    if (!['http:', 'https:'].includes(new URL(target).protocol)) throw new Error(`Invalid ${role} daemon URL`);
  }
  return { plugins: [daemonBridge(targets)], server: { strictPort: true }, preview: { strictPort: true } };
});
