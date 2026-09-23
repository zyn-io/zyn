// Two pages, one origin: the wallet and the market share the kept key, the
// chain route and the content policy, so neither is a second place to trust.
import { defineConfig } from 'vite';

const devProxy = () => (process.env.NAP_DEV_RPC
  ? { proxy: { '/rpc': { target: process.env.NAP_DEV_RPC, changeOrigin: true, rewrite: () => '/' } } }
  : {});

export default defineConfig({
  build: {
    rollupOptions: {
      input: { wallet: 'index.html', market: 'market.html' },
    },
  },
  // Local development only: `NAP_DEV_RPC=http://127.0.0.1:8399 npm run dev`
  // points /rpc at a node on this machine, the way the Pages function does in
  // production. Unset, there is no proxy and nothing changes.
  // `vite preview` too: the dev server injects styles inline, which the
  // shipped CSP rightly blocks, so the built pages are what to look at.
  server: devProxy(),
  preview: devProxy(),
});
