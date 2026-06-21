import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import {
  ModelPickerPanel,
  formatCapabilityLabel,
  OLLAMA_LIBRARY_URL,
  OLLAMA_PILL_TOOLTIP,
} from '../ModelPickerPanel';
import type { ModelCapabilitiesMap } from '../../types/model';
import {
  BUILTIN_NO_MODELS_MESSAGE,
  OPENAI_NO_MODEL_MESSAGE,
} from '../../utils/capabilityConflicts';
import { invoke } from '@tauri-apps/api/core';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

const MODELS = ['gemma4:e2b', 'qwen2.5:7b', 'llama3.2:3b'];

function renderPanel(
  overrides: Partial<React.ComponentProps<typeof ModelPickerPanel>> = {},
) {
  const props: React.ComponentProps<typeof ModelPickerPanel> = {
    models: MODELS,
    activeModel: 'gemma4:e2b',
    onSelect: vi.fn(),
    ...overrides,
  };
  return { props, ...render(<ModelPickerPanel {...props} />) };
}

describe('ModelPickerPanel', () => {
  it('renders filter input', () => {
    renderPanel();
    expect(screen.getByPlaceholderText(/filter models/i)).toBeInTheDocument();
  });

  it('shows all models on first render', () => {
    renderPanel();
    for (const model of MODELS) {
      expect(screen.getByRole('option', { name: model })).toBeInTheDocument();
    }
  });

  const BUILTIN_ID = 'unsloth/Qwen3.5-9B-GGUF:Qwen3.5-9B-Q4_K_M.gguf';

  it('renders the friendly display name for ids that have one', () => {
    renderPanel({
      models: [BUILTIN_ID],
      activeModel: null,
      displayNames: { [BUILTIN_ID]: 'Qwen3.5 9B' },
    });
    expect(
      screen.getByRole('option', { name: 'Qwen3.5 9B' }),
    ).toBeInTheDocument();
    expect(screen.queryByText(BUILTIN_ID)).not.toBeInTheDocument();
    // The truncated name carries the full label as a native hover tooltip.
    expect(screen.getByText('Qwen3.5 9B')).toHaveAttribute(
      'title',
      'Qwen3.5 9B',
    );
  });

  it('falls back to the id when no display name is given', () => {
    renderPanel({
      models: ['llama3.2:3b'],
      activeModel: null,
      displayNames: {},
    });
    expect(
      screen.getByRole('option', { name: 'llama3.2:3b' }),
    ).toBeInTheDocument();
  });

  it('filters by the friendly display name, not just the id', () => {
    renderPanel({
      models: [BUILTIN_ID, 'llama3.2:3b'],
      activeModel: null,
      displayNames: { [BUILTIN_ID]: 'Qwen3.5 9B' },
    });
    fireEvent.change(screen.getByPlaceholderText(/filter models/i), {
      target: { value: 'qwen3.5 9b' },
    });
    expect(
      screen.getByRole('option', { name: 'Qwen3.5 9B' }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole('option', { name: 'llama3.2:3b' }),
    ).not.toBeInTheDocument();
  });

  it('marks active model with aria-selected true, others false', () => {
    renderPanel({ activeModel: 'qwen2.5:7b' });
    expect(screen.getByRole('option', { name: 'qwen2.5:7b' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    expect(screen.getByRole('option', { name: 'gemma4:e2b' })).toHaveAttribute(
      'aria-selected',
      'false',
    );
    expect(screen.getByRole('option', { name: 'llama3.2:3b' })).toHaveAttribute(
      'aria-selected',
      'false',
    );
  });

  it('shows visible checkmark on active model, hidden on others', () => {
    renderPanel({ activeModel: 'gemma4:e2b' });
    const activeItem = screen.getByRole('option', { name: 'gemma4:e2b' });
    const inactiveItem = screen.getByRole('option', { name: 'qwen2.5:7b' });
    const activeCheck = activeItem.querySelector('svg')!;
    const inactiveCheck = inactiveItem.querySelector('svg')!;
    expect((activeCheck as SVGElement).style.opacity).toBe('1');
    expect((inactiveCheck as SVGElement).style.opacity).toBe('0');
  });

  it('calls onSelect with slug when row clicked', () => {
    const onSelect = vi.fn();
    renderPanel({ onSelect });
    fireEvent.click(screen.getByRole('option', { name: 'qwen2.5:7b' }));
    expect(onSelect).toHaveBeenCalledWith('qwen2.5:7b');
    expect(onSelect).toHaveBeenCalledTimes(1);
  });

  it('filters models as user types', () => {
    renderPanel();
    fireEvent.change(screen.getByPlaceholderText(/filter models/i), {
      target: { value: 'qwen' },
    });
    expect(
      screen.getByRole('option', { name: 'qwen2.5:7b' }),
    ).toBeInTheDocument();
    expect(screen.queryByRole('option', { name: 'gemma4:e2b' })).toBeNull();
    expect(screen.queryByRole('option', { name: 'llama3.2:3b' })).toBeNull();
  });

  it('shows no-models-found message when filter matches nothing', () => {
    renderPanel();
    fireEvent.change(screen.getByPlaceholderText(/filter models/i), {
      target: { value: 'zzz' },
    });
    expect(screen.getByText(/no models found/i)).toBeInTheDocument();
    expect(screen.queryByRole('option')).toBeNull();
  });

  it('restores full list when filter is cleared', () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/filter models/i);
    fireEvent.change(input, { target: { value: 'qwen' } });
    fireEvent.change(input, { target: { value: '' } });
    for (const model of MODELS) {
      expect(screen.getByRole('option', { name: model })).toBeInTheDocument();
    }
  });

  it('shows the ollama-pull empty state when models list is empty', () => {
    // Empty state copy mirrors S2 in `getEnvironmentMessage`: route the
    // user to `ollama pull <model>` rather than implying anything is
    // wrong with the picker itself.
    renderPanel({ models: [] });
    const empty = screen.getByTestId('model-picker-empty');
    expect(empty).toBeInTheDocument();
    expect(empty.textContent).toContain('No models installed');
    expect(empty.textContent).toContain('ollama pull <model>');
    expect(screen.queryByRole('option')).toBeNull();
  });

  it('routes a builtin user to the Settings download picker in the empty state', () => {
    renderPanel({ models: [], providerKind: 'builtin' });
    const empty = screen.getByTestId('model-picker-empty');
    expect(empty.textContent).toBe(BUILTIN_NO_MODELS_MESSAGE);
    expect(empty.textContent).not.toContain('ollama pull');
  });

  it('routes an openai user to the Settings provider model in the empty state', () => {
    renderPanel({ models: [], providerKind: 'openai' });
    const empty = screen.getByTestId('model-picker-empty');
    expect(empty.textContent).toBe(OPENAI_NO_MODEL_MESSAGE);
    expect(empty.textContent).not.toContain('ollama pull');
  });

  it('keeps the ollama-pull empty state when providerKind is ollama', () => {
    renderPanel({ models: [], providerKind: 'ollama' });
    const empty = screen.getByTestId('model-picker-empty');
    expect(empty.textContent).toContain('ollama pull <model>');
  });

  it('hides the Browse Ollama pill for non-ollama providers', () => {
    renderPanel({ providerKind: 'builtin' });
    expect(screen.queryByTestId('model-picker-ollama-link')).toBeNull();
  });

  it('renders no row as active when activeModel is null', () => {
    // S2/S3: the chip stays clickable with a null active model. The panel
    // must accept null without inventing a default and simply mark no row
    // as aria-selected.
    renderPanel({ activeModel: null as unknown as string });
    for (const model of MODELS) {
      const option = screen.getByRole('option', { name: model });
      expect(option).toHaveAttribute('aria-selected', 'false');
    }
  });

  it('marks the filter input as an aria-activedescendant combobox', () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/filter models/i);
    expect(input).toHaveAttribute('role', 'combobox');
    expect(input).toHaveAttribute(
      'aria-activedescendant',
      expect.stringContaining('option-0'),
    );
  });

  it('ArrowDown advances the highlighted descendant', () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/filter models/i);
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    expect(input).toHaveAttribute(
      'aria-activedescendant',
      expect.stringContaining('option-1'),
    );
  });

  it('ArrowUp wraps to the last row from the first', () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/filter models/i);
    fireEvent.keyDown(input, { key: 'ArrowUp' });
    expect(input).toHaveAttribute(
      'aria-activedescendant',
      expect.stringContaining(`option-${MODELS.length - 1}`),
    );
  });

  it('Home/End jump to the first and last rows', () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/filter models/i);
    fireEvent.keyDown(input, { key: 'End' });
    expect(input).toHaveAttribute(
      'aria-activedescendant',
      expect.stringContaining(`option-${MODELS.length - 1}`),
    );
    fireEvent.keyDown(input, { key: 'Home' });
    expect(input).toHaveAttribute(
      'aria-activedescendant',
      expect.stringContaining('option-0'),
    );
  });

  it('Enter commits the highlighted row via onSelect', () => {
    const onSelect = vi.fn();
    renderPanel({ onSelect });
    const input = screen.getByPlaceholderText(/filter models/i);
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onSelect).toHaveBeenCalledWith('qwen2.5:7b');
  });

  it('Escape fires onClose when provided', () => {
    const onClose = vi.fn();
    renderPanel({ onClose });
    const input = screen.getByPlaceholderText(/filter models/i);
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('Escape without onClose is a safe no-op', () => {
    const onSelect = vi.fn();
    renderPanel({ onSelect });
    const input = screen.getByPlaceholderText(/filter models/i);
    fireEvent.keyDown(input, { key: 'Escape' });
    // Escape must never select a model.
    expect(onSelect).not.toHaveBeenCalled();
    // Focus must remain on the filter input.
    expect(document.activeElement).toBe(screen.getByRole('combobox'));
    // Filter value must be unchanged (Escape does not clear input).
    expect((document.activeElement as HTMLInputElement).value).toBe('');
  });

  it('keyboard nav on empty filter result is a safe no-op', () => {
    const onSelect = vi.fn();
    renderPanel({ onSelect });
    const input = screen.getByPlaceholderText(/filter models/i);
    fireEvent.change(input, { target: { value: 'zzz' } });
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    fireEvent.keyDown(input, { key: 'ArrowUp' });
    fireEvent.keyDown(input, { key: 'Home' });
    fireEvent.keyDown(input, { key: 'End' });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onSelect).not.toHaveBeenCalled();
    expect(input).not.toHaveAttribute('aria-activedescendant');
  });

  it('clamps highlighted index when the filtered list shrinks', () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/filter models/i);
    fireEvent.keyDown(input, { key: 'End' });
    // Narrow the visible set to one row; the activedescendant must clamp to 0.
    fireEvent.change(input, { target: { value: 'qwen' } });
    expect(input).toHaveAttribute(
      'aria-activedescendant',
      expect.stringContaining('option-0'),
    );
  });

  it('mouse-over updates the highlighted descendant', () => {
    renderPanel();
    fireEvent.mouseEnter(screen.getByRole('option', { name: 'llama3.2:3b' }));
    const input = screen.getByPlaceholderText(/filter models/i);
    expect(input).toHaveAttribute(
      'aria-activedescendant',
      expect.stringContaining('option-2'),
    );
  });

  it('shows nudge inline in header when not compact', () => {
    renderPanel();
    expect(
      screen.getByText('Larger models answer better.'),
    ).toBeInTheDocument();
  });

  it('shows nudge as bottom footer when compact', () => {
    renderPanel({ compact: true });
    expect(
      screen.getByText('Larger models answer better.'),
    ).toBeInTheDocument();
  });

  it('hides compact footer nudge when no models are installed', () => {
    renderPanel({ compact: true, models: [] });
    expect(
      screen.queryByText('Larger models answer better.'),
    ).not.toBeInTheDocument();
  });

  it('ignores other keys without preventing default or firing handlers', () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();
    renderPanel({ onSelect, onClose });
    const input = screen.getByPlaceholderText(/filter models/i);
    fireEvent.keyDown(input, { key: 'a' });
    expect(onSelect).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it('renders capability labels per row when capabilities prop is provided', () => {
    const capabilities: ModelCapabilitiesMap = {
      'gemma4:e2b': {
        vision: true,
        thinking: false,
      },
      'qwen2.5:7b': {
        vision: false,
        thinking: true,
      },
      'llama3.2:3b': {
        vision: false,
        thinking: false,
      },
    };
    renderPanel({ capabilities });
    // Every row leads with "text" (every chat model handles text), then
    // appends vision/thinking when supported. Plain models render just "text".
    const labels = screen.getAllByTestId('model-capability-label');
    expect(labels.length).toBe(3);
    expect(labels[0]).toHaveTextContent('text · vision');
    expect(labels[1]).toHaveTextContent('text · thinking');
    expect(labels[2]).toHaveTextContent('text');
  });

  it('row aria-label includes capability summary when present', () => {
    const capabilities: ModelCapabilitiesMap = {
      'gemma4:e2b': {
        vision: true,
        thinking: false,
      },
    };
    renderPanel({ models: ['gemma4:e2b'], capabilities });
    const row = screen.getByRole('option', {
      name: /gemma4:e2b, text, vision/i,
    });
    expect(row).toBeInTheDocument();
  });

  it('renders the "Always thinks" badge only for reasoningAlways models', () => {
    const capabilities: ModelCapabilitiesMap = {
      'gemma4:e2b': { vision: true, thinking: false },
      'qwen2.5:7b': { vision: false, thinking: true, reasoningAlways: true },
      'llama3.2:3b': { vision: false, thinking: false },
    };
    renderPanel({ capabilities });
    const badges = screen.getAllByTestId('always-thinks-badge');
    expect(badges).toHaveLength(1);
    expect(badges[0]).toHaveTextContent('Always thinks');
  });
});

describe('formatCapabilityLabel', () => {
  it('returns null when capabilities map is undefined', () => {
    expect(formatCapabilityLabel(undefined, 'x')).toBeNull();
  });

  it('returns null when the model is not in the map', () => {
    expect(formatCapabilityLabel({}, 'x')).toBeNull();
  });

  it('returns "text" for plain models with no surface-worthy capabilities', () => {
    const map: ModelCapabilitiesMap = {
      x: { vision: false, thinking: false },
    };
    expect(formatCapabilityLabel(map, 'x')).toBe('text');
  });

  it('leads with "text" and appends every supported flag, joined with " · "', () => {
    const map: ModelCapabilitiesMap = {
      x: { vision: true, thinking: true },
    };
    expect(formatCapabilityLabel(map, 'x')).toBe('text · vision · thinking');
  });

  it('appends "vision" after the leading "text" when only vision is present', () => {
    const map: ModelCapabilitiesMap = {
      x: { vision: true, thinking: false },
    };
    expect(formatCapabilityLabel(map, 'x')).toBe('text · vision');
  });

  it('appends "thinking" after the leading "text" when only thinking is present', () => {
    const map: ModelCapabilitiesMap = {
      x: { vision: false, thinking: true },
    };
    expect(formatCapabilityLabel(map, 'x')).toBe('text · thinking');
  });
});

describe('ModelPickerPanel "Browse Ollama" pill', () => {
  it('renders the Browse Ollama button next to the filter input', () => {
    render(
      <ModelPickerPanel
        models={MODELS}
        activeModel="gemma4:e2b"
        onSelect={vi.fn()}
      />,
    );
    const pill = screen.getByTestId('model-picker-ollama-link');
    expect(pill).toBeInTheDocument();
    expect(pill).toHaveTextContent(/Browse Ollama/i);
    expect(pill).toHaveAttribute('aria-label', 'Browse Ollama models');
  });

  it('opens the Ollama library URL via open_url when clicked', () => {
    render(
      <ModelPickerPanel
        models={MODELS}
        activeModel="gemma4:e2b"
        onSelect={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByTestId('model-picker-ollama-link'));
    expect(invoke).toHaveBeenCalledWith('open_url', {
      url: OLLAMA_LIBRARY_URL,
    });
  });

  it('exports a stable Ollama library URL constant', () => {
    expect(OLLAMA_LIBRARY_URL).toBe('https://ollama.com/library');
  });

  it('exports a stable tooltip body constant', () => {
    expect(OLLAMA_PILL_TOOLTIP).toMatch(/Browse and pull any model on Ollama/i);
    expect(OLLAMA_PILL_TOOLTIP).toMatch(/Thuki auto-detects it/i);
  });

  it('uses no em dashes in the tooltip body', () => {
    expect(OLLAMA_PILL_TOOLTIP).not.toContain('—');
  });

  it('drops the "Ollama" word in compact mode so the chip drawer stays uncluttered', () => {
    render(
      <ModelPickerPanel
        models={MODELS}
        activeModel="gemma4:e2b"
        onSelect={vi.fn()}
        compact
      />,
    );
    const pill = screen.getByTestId('model-picker-ollama-link');
    expect(pill).toHaveTextContent(/^Browse$/);
    expect(pill).not.toHaveTextContent(/Ollama/);
    // Aria-label still spells it out for assistive tech.
    expect(pill).toHaveAttribute('aria-label', 'Browse Ollama models');
  });
});
