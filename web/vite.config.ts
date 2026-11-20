import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const certDir = path.resolve(__dirname, '.certs');
const keyPath = path.join(certDir, 'key.pem');
const certPath = path.join(certDir, 'cert.pem');
const hasCert = fs.existsSync(keyPath) && fs.existsSync(certPath);

export default defineConfig({
  server: {
    port: 5173,
    open: false,
    ...(hasCert
      ? {
          https: {
            key: fs.readFileSync(keyPath),
            cert: fs.readFileSync(certPath),
          },
        }
      : {}),
  },
  build: {
    target: 'es2022',
  },
  worker: {
    format: 'es',
  },
});
