import { render, act, screen } from '@testing-library/react';
import { describe, it, expect, beforeEach } from 'vitest';
import {
  ConfigProvider,
  ConfigProviderForTest,
  DEFAULT_CONFIG,
  useConfig,
  type AppConfig,
} from '../ConfigContext';
import { invoke } from '../../testUtils/mocks/tauri';

function Probe() {
  const config = useConfig();
  return (
    <>
      <div data-testid="active-model">{config.model.active}</div>
      <div data-testid="overlay-width">{config.window.overlayWidth}</div>
      <div data-testid="max-display-lines">{config.quote.maxDisplayLines}</div>
      <div data-testid="system-prompt">{config.prompt.system}</div>
      <div data-testid="hide-delay">{config.window.hideCommitDelayMs}</div>
      <div data-testid="schema-version">{config.schemaVersion}</div>
    </>
  );
}

describe('ConfigContext', () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  describe('useConfig fallback', () => {
    it('returns DEFAULT_CONFIG when no provider is in the tree', () => {
      render(<Probe />);
      expect(screen.getByTestId('active-model').textContent).toBe(
        DEFAULT_CONFIG.model.active,
      );
      expect(screen.getByTestId('overlay-width').textContent).toBe(
        String(DEFAULT_CONFIG.window.overlayWidth),
      );
      expect(screen.getByTestId('max-display-lines').textContent).toBe(
        String(DEFAULT_CONFIG.quote.maxDisplayLines),
      );
    });
  });

  describe('ConfigProviderForTest', () => {
    it('provides the supplied value to descendants', () => {
      const custom: AppConfig = {
        ...DEFAULT_CONFIG,
        model: {
          active: 'custom:model',
          available: ['custom:model', 'other:model'],
          ollamaUrl: 'http://example.test:11434',
        },
      };
      render(
        <ConfigProviderForTest value={custom}>
          <Probe />
        </ConfigProviderForTest>,
      );
      expect(screen.getByTestId('active-model').textContent).toBe(
        'custom:model',
      );
    });
  });

  describe('ConfigProvider', () => {
    it('hydrates from the backend and transforms snake_case to camelCase', async () => {
      invoke.mockResolvedValueOnce({
        schema_version: 1,
        model: {
          available: ['gemma4:e4b', 'gemma4:e2b'],
          ollama_url: 'http://127.0.0.1:11434',
        },
        prompt: { system: 'custom base prompt' },
        window: {
          overlay_width: 800,
          collapsed_height: 100,
          max_chat_height: 700,
          hide_commit_delay_ms: 400,
        },
        quote: {
          max_display_lines: 6,
          max_display_chars: 500,
          max_context_length: 8192,
        },
      });

      render(
        <ConfigProvider>
          <Probe />
        </ConfigProvider>,
      );
      // Let the useEffect + promise resolution flush.
      await act(async () => {});

      expect(screen.getByTestId('active-model').textContent).toBe('gemma4:e4b');
      expect(screen.getByTestId('overlay-width').textContent).toBe('800');
      expect(screen.getByTestId('max-display-lines').textContent).toBe('6');
      expect(screen.getByTestId('system-prompt').textContent).toBe(
        'custom base prompt',
      );
      expect(screen.getByTestId('hide-delay').textContent).toBe('400');
      expect(screen.getByTestId('schema-version').textContent).toBe('1');
    });

    it('falls back to DEFAULT_CONFIG when invoke returns nullish', async () => {
      invoke.mockResolvedValueOnce(undefined);

      render(
        <ConfigProvider>
          <Probe />
        </ConfigProvider>,
      );
      await act(async () => {});

      expect(screen.getByTestId('active-model').textContent).toBe(
        DEFAULT_CONFIG.model.active,
      );
      expect(screen.getByTestId('overlay-width').textContent).toBe(
        String(DEFAULT_CONFIG.window.overlayWidth),
      );
    });

    it('falls back to DEFAULT_CONFIG when the available list is empty', async () => {
      // Edge case: Rust loader always prevents this, but the frontend transform
      // should still produce a usable `active` (empty string) from an empty list.
      invoke.mockResolvedValueOnce({
        schema_version: 1,
        model: { available: [], ollama_url: 'http://127.0.0.1:11434' },
        prompt: { system: '' },
        window: {
          overlay_width: 600,
          collapsed_height: 80,
          max_chat_height: 648,
          hide_commit_delay_ms: 350,
        },
        quote: {
          max_display_lines: 4,
          max_display_chars: 300,
          max_context_length: 4096,
        },
      });

      render(
        <ConfigProvider>
          <Probe />
        </ConfigProvider>,
      );
      await act(async () => {});

      expect(screen.getByTestId('active-model').textContent).toBe('');
    });

    it('falls back to DEFAULT_CONFIG when invoke rejects', async () => {
      invoke.mockRejectedValueOnce(new Error('IPC bridge unavailable'));

      render(
        <ConfigProvider>
          <Probe />
        </ConfigProvider>,
      );
      await act(async () => {});

      expect(screen.getByTestId('active-model').textContent).toBe(
        DEFAULT_CONFIG.model.active,
      );
      expect(screen.getByTestId('overlay-width').textContent).toBe(
        String(DEFAULT_CONFIG.window.overlayWidth),
      );
    });

    it('renders nothing before the initial invoke resolves', () => {
      invoke.mockImplementation(
        () => new Promise<never>(() => {}), // pending forever
      );
      const { container } = render(
        <ConfigProvider>
          <div data-testid="child">child</div>
        </ConfigProvider>,
      );
      expect(container.textContent).toBe('');
    });
  });
});
