import { defineConfig } from 'vite'
import preact from '@preact/preset-vite'
import viteCompression from 'vite-plugin-compression'

// For local UI development against a real device, proxy its API:
//   DEVICE=<device-ip> npm run dev
const device = process.env.DEVICE

// https://vite.dev/config/
export default defineConfig({
  server: device ? { proxy: { '/api': `http://${device}` } } : undefined,
  plugins: [
    preact(),
    viteCompression({
      algorithm: 'gzip',
      ext: '.gz',
      threshold: 0,
    }),
  ],
  build: {
    rollupOptions: {
      output: {
        entryFileNames: "assets/bundle.js",
        assetFileNames: "assets/bundle.[ext]",
        chunkFileNames: "assets/bundle-[hash].js",
      }
    }
  }
})
