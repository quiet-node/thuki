import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import { resolve } from 'path';

export default defineConfig({
  plugins: [react()],
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['src/testUtils/setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
    coverage: {
      provider: 'v8',
      include: ['src/**/*.{ts,tsx}'],
      exclude: ['src/vite-env.d.ts', 'src/testUtils/**', 'src/**/*.test.{ts,tsx}', 'src/types/**'],
      thresholds: {
        lines: 100,
        functions: 100,
        branches: 100,
        statements: 100,
      },
    },
    alias: {
      // Test-only module aliases: replace Tauri/Framer Motion with mocks
      // These do NOT affect production code — only tests use this config
      '@tauri-apps/api/core': resolve(__dirname, 'src/testUtils/mocks/tauri.ts'),
      '@tauri-apps/api/event': resolve(__dirname, 'src/testUtils/mocks/tauri.ts'),
      '@tauri-apps/api/window': resolve(
        __dirname,
        'src/testUtils/mocks/tauri-window.ts',
      ),
      '@tauri-apps/api/dpi': resolve(
        __dirname,
        'src/testUtils/mocks/tauri-window.ts',
      ),
      'framer-motion': resolve(__dirname, 'src/testUtils/mocks/framer-motion.tsx'),
    },
  },
});
