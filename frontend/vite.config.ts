import { defineConfig } from 'vite'
import preact from '@preact/preset-vite'
import viteCompression from 'vite-plugin-compression'

// https://vite.dev/config/
export default defineConfig({
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
