import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import { resolve } from 'path';

export default defineConfig({
  plugins: [react()],
  test: {
    environment: 'happy-dom',
    setupFiles: ['src/test/setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
    coverage: {
      provider: 'v8',
      include: ['src/**/*.{ts,tsx}'],
      exclude: ['src/vite-env.d.ts', 'src/test/**', 'src/**/*.test.{ts,tsx}'],
      thresholds: {
        lines: 100,
        functions: 100,
        branches: 100,
        statements: 100,
      },
    },
    alias: {
      '@tauri-apps/api/core': resolve(__dirname, 'src/test/mocks/tauri.ts'),
      '@tauri-apps/api/event': resolve(__dirname, 'src/test/mocks/tauri.ts'),
      '@tauri-apps/api/window': resolve(
        __dirname,
        'src/test/mocks/tauri-window.ts',
      ),
      '@tauri-apps/api/dpi': resolve(
        __dirname,
        'src/test/mocks/tauri-window.ts',
      ),
      'framer-motion': resolve(__dirname, 'src/test/mocks/framer-motion.tsx'),
    },
  },
});
