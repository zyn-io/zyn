// Two pages, one origin: the wallet and the market share the kept key, the
// chain route and the content policy, so neither is a second place to trust.
import { defineConfig } from 'vite';

const devProxy = () => {
  const proxy = {};
  if (process.env.NAP_DEV_RPC) proxy['/rpc'] = { target: process.env.NAP_DEV_RPC, changeOrigin: true, rewrite: () => '/' };
  // A block server's HTTP door (`ZYN_LIGHTD_HTTP`), for the Zcash wallet.
  if (process.env.NAP_DEV_ZEC) proxy['/zec'] = { target: process.env.NAP_DEV_ZEC, changeOrigin: true, rewrite: () => '/' };
  return Object.keys(proxy).length ? { proxy } : {};
};

// `vite build --mode extension` (or `--mode mobile`) builds the same pages
// for the browser extension (apps/extension) or the phone app (apps/nap-mobile): the endpoints stay on Nap's server
// (`VITE_NAP_API` in .env.extension), so each page's policy admits that
// origin too, and the web app's install manifest does not apply.
const extensionPages = (mode) => ({
  name: 'nap-extension-pages',
  transformIndexHtml(html) {
    if (mode !== 'extension' && mode !== 'mobile') return html;
    const api = new URL(process.env.VITE_NAP_API || 'https://nap.cavebro.win').origin;
    return html
      .replace(/connect-src 'self'/, `connect-src 'self' ${api}`)
      .replace(/\s*<link rel="manifest"[^>]*>/, '');
  },
});

export default defineConfig(({ mode }) => ({
  plugins: [extensionPages(mode)],
  build: {
    rollupOptions: {
      input: { wallet: 'index.html', market: 'market.html', connect: 'connect.html', bridge: 'bridge.html' },
    },
  },
  // Local development only: `NAP_DEV_RPC=http://127.0.0.1:8399 npm run dev`
  // points /rpc at a node on this machine, the way the Pages function does in
  // production; `NAP_DEV_ZEC=http://127.0.0.1:8095` does the same for /zec. Unset, there is no proxy and nothing changes.
  // `vite preview` too: the dev server injects styles inline, which the
  // shipped CSP rightly blocks, so the built pages are what to look at.
  server: devProxy(),
  preview: devProxy(),
}));
