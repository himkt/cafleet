import { defineConfig } from 'vite'
import react, { reactCompilerPreset } from '@vitejs/plugin-react'
import babel from '@rolldown/plugin-babel'
import tailwindcss from '@tailwindcss/vite'

// https://vite.dev/config/
export default defineConfig({
  base: '/ui/',
  plugins: [
    react(),
    babel({ presets: [reactCompilerPreset()] }),
    tailwindcss(),
  ],
  build: {
    outDir: '../hikyaku/src/hikyaku/webui',
    emptyOutDir: true,
  },
  server: {
    proxy: {
      '/ui/api': {
        target: 'http://localhost:8000',
        changeOrigin: true,
      },
    },
  },
})
