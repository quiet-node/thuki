/**
 * Tests for the Behavior-tab "Models allowed over the memory limit" section:
 * name resolution from installed models, the short-sha orphan fallback, the
 * Remove path (calls `forget_model_memory_fit` with the sha and lifts the
 * returned config), and the empty-list hide.
 */

import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { invoke } from '@tauri-apps/api/core';

import { DismissedMemoryFitSection } from '../DismissedMemoryFitSection';
import type { InstalledModel } from '../../../types/starter';
import type { RawAppConfig } from '../../types';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

const SHA_A = 'a'.repeat(64);
const SHA_B = 'b'.repeat(64);

const INSTALLED: InstalledModel[] = [
  {
    id: 'org/gemma:gemma.gguf',
    sha256: SHA_A,
    display_name: 'Gemma 3 4B',
    size_bytes: 1,
    quant: 'Q4_K_M',
  },
];

/** Base config with a configurable dismissed list. */
function configWith(dismissed: string[]): RawAppConfig {
  return {
    inference: {
      active_provider: 'builtin',
      keep_warm_inactivity_minutes: 0,
      num_ctx: 16384,
      providers: [],
    },
    prompt: { system: '' },
    window: {
      overlay_width: 600,
      max_chat_height: 648,
      max_images: 3,
      text_base_px: 15,
      text_line_height: 1.5,
      text_letter_spacing_px: 0,
      text_font_weight: 500,
    },
    quote: {
      max_display_lines: 4,
      max_display_chars: 300,
      max_context_length: 4096,
    },
    behavior: {
      auto_replace: false,
      auto_close: false,
      auto_search: true,
      search_notice_acknowledged: false,
      auto_save_conversations: true,
      history_retention_days: -1,
      auto_save_notice_acknowledged: false,
      dismissed_memory_fit_models: dismissed,
    },
    debug: { trace_enabled: false, trace_retention_days: 7 },
  };
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === 'list_installed_models') return Promise.resolve(INSTALLED);
    return Promise.resolve(configWith([]));
  });
});

afterEach(() => {
  vi.clearAllMocks();
});

describe('DismissedMemoryFitSection', () => {
  it('renders nothing when the dismissed list is empty', () => {
    const { container } = render(
      <DismissedMemoryFitSection config={configWith([])} onSaved={vi.fn()} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it('renders one row per entry, resolving installed names and short-sha orphans', async () => {
    render(
      <DismissedMemoryFitSection
        config={configWith([SHA_A, SHA_B])}
        onSaved={vi.fn()}
      />,
    );
    // Installed sha resolves to its display name.
    expect(await screen.findByText('Gemma 3 4B')).toBeInTheDocument();
    // Orphaned sha (not installed) falls back to a short-sha label.
    expect(screen.getByText(`${SHA_B.slice(0, 8)}…`)).toBeInTheDocument();
    expect(screen.getAllByRole('button', { name: 'Remove' })).toHaveLength(2);
  });

  it('Remove calls forget_model_memory_fit with the sha and lifts the config', async () => {
    const onSaved = vi.fn();
    const next = configWith([SHA_B]);
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'list_installed_models') return Promise.resolve(INSTALLED);
      if (cmd === 'forget_model_memory_fit') return Promise.resolve(next);
      return Promise.resolve(configWith([]));
    });

    render(
      <DismissedMemoryFitSection
        config={configWith([SHA_A, SHA_B])}
        onSaved={onSaved}
      />,
    );
    await screen.findByText('Gemma 3 4B');
    fireEvent.click(screen.getAllByRole('button', { name: 'Remove' })[0]);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('forget_model_memory_fit', {
        modelSha: SHA_A,
      });
    });
    await waitFor(() => expect(onSaved).toHaveBeenCalledWith(next));
  });

  it('keeps the row when the remove command rejects', async () => {
    const onSaved = vi.fn();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'list_installed_models') return Promise.resolve(INSTALLED);
      if (cmd === 'forget_model_memory_fit') {
        return Promise.reject(new Error('disk full'));
      }
      return Promise.resolve(configWith([]));
    });

    render(
      <DismissedMemoryFitSection
        config={configWith([SHA_A])}
        onSaved={onSaved}
      />,
    );
    await screen.findByText('Gemma 3 4B');
    fireEvent.click(screen.getByRole('button', { name: 'Remove' }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('forget_model_memory_fit', {
        modelSha: SHA_A,
      });
    });
    // Best-effort: onSaved never fires and the row remains.
    expect(onSaved).not.toHaveBeenCalled();
    expect(screen.getByText('Gemma 3 4B')).toBeInTheDocument();
  });

  it('falls back to short-sha labels when the installed list is not an array', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'list_installed_models') return Promise.resolve(null);
      return Promise.resolve(configWith([]));
    });

    render(
      <DismissedMemoryFitSection
        config={configWith([SHA_A])}
        onSaved={vi.fn()}
      />,
    );
    expect(
      await screen.findByText(`${SHA_A.slice(0, 8)}…`),
    ).toBeInTheDocument();
  });

  it('falls back to short-sha labels when the installed fetch fails', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'list_installed_models') {
        return Promise.reject(new Error('manifest unreadable'));
      }
      return Promise.resolve(configWith([]));
    });

    render(
      <DismissedMemoryFitSection
        config={configWith([SHA_A])}
        onSaved={vi.fn()}
      />,
    );
    expect(
      await screen.findByText(`${SHA_A.slice(0, 8)}…`),
    ).toBeInTheDocument();
  });
});
