import { fireEvent, render, screen } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { ErrorCard, INSUFFICIENT_MEMORY_CONSEQUENCE } from '../ErrorCard';

describe('ErrorCard', () => {
  it('renders the title (first line of message)', () => {
    render(
      <ErrorCard
        kind="EngineUnreachable"
        message={"Ollama isn't running\nStart Ollama and try again."}
      />,
    );
    expect(screen.getByText("Ollama isn't running")).toBeInTheDocument();
  });

  it('renders the subtitle (second line of message)', () => {
    render(
      <ErrorCard
        kind="EngineUnreachable"
        message={"Ollama isn't running\nStart Ollama and try again."}
      />,
    );
    expect(screen.getByText('Start Ollama and try again.')).toBeInTheDocument();
  });

  it('renders only title when message has no newline', () => {
    render(<ErrorCard kind="Other" message="Something went wrong" />);
    expect(screen.getByText('Something went wrong')).toBeInTheDocument();
  });

  it('applies red accent bar for EngineUnreachable', () => {
    const { container } = render(
      <ErrorCard
        kind="EngineUnreachable"
        message={"Ollama isn't running\nStart Ollama."}
      />,
    );
    const bar = container.querySelector('[data-error-bar]');
    expect(bar).not.toBeNull();
    expect(bar?.getAttribute('data-kind')).toBe('EngineUnreachable');
  });

  it('applies red accent bar for EngineStartFailed', () => {
    const { container } = render(
      <ErrorCard
        kind="EngineStartFailed"
        message={
          'Engine failed to start\nThe bundled sidecar crashed before becoming healthy.'
        }
      />,
    );
    const bar = container.querySelector('[data-error-bar]');
    expect(bar).not.toBeNull();
    expect(bar?.getAttribute('data-kind')).toBe('EngineStartFailed');
    // JSDOM normalizes hex to rgb; assert the same red family as EngineUnreachable.
    expect((bar as HTMLElement | null)?.style.background).toBe(
      'rgb(239, 68, 68)',
    );
  });

  it('applies amber accent bar for ModelNotFound', () => {
    const { container } = render(
      <ErrorCard
        kind="ModelNotFound"
        message={'Model not found\nRun: ollama pull gemma3:4b'}
      />,
    );
    const bar = container.querySelector('[data-error-bar]');
    expect(bar?.getAttribute('data-kind')).toBe('ModelNotFound');
  });

  it('applies amber accent bar for ModelUnsupported', () => {
    const { container } = render(
      <ErrorCard
        kind="ModelUnsupported"
        message={
          "Unsupported model\nThuki's engine doesn't support this arch yet."
        }
      />,
    );
    const bar = container.querySelector('[data-error-bar]');
    expect(bar?.getAttribute('data-kind')).toBe('ModelUnsupported');
    // JSDOM normalizes hex to rgb; assert the amber family (same as ModelNotFound).
    expect((bar as HTMLElement | null)?.style.background).toBe(
      'rgb(245, 158, 11)',
    );
  });

  it('renders the ModelUnsupported copy (title and subtitle)', () => {
    render(
      <ErrorCard
        kind="ModelUnsupported"
        message={
          "Unsupported model\nThuki's engine doesn't support this arch yet. Try another model. Engine improves over time and may support it down the road."
        }
      />,
    );
    expect(screen.getByText('Unsupported model')).toBeInTheDocument();
    expect(
      screen.getByText(
        "Thuki's engine doesn't support this arch yet. Try another model. Engine improves over time and may support it down the road.",
      ),
    ).toBeInTheDocument();
  });

  it('applies neutral accent bar for Other', () => {
    const { container } = render(
      <ErrorCard kind="Other" message={'Something went wrong\nHTTP 500'} />,
    );
    const bar = container.querySelector('[data-error-bar]');
    expect(bar?.getAttribute('data-kind')).toBe('Other');
  });

  it('renders model pull command as code in subtitle', () => {
    const { container } = render(
      <ErrorCard
        kind="ModelNotFound"
        message={'Model not found\nRun: ollama pull gemma3:4b in a terminal.'}
      />,
    );
    const code = container.querySelector('code');
    expect(code).not.toBeNull();
    expect(code?.textContent).toContain('ollama pull gemma3:4b');
  });

  // The strings below pin the backend's provider-aware copy contract:
  // Rust owns the wording, ErrorCard renders it verbatim.

  it('renders the builtin EngineUnreachable copy (title and subtitle)', () => {
    render(
      <ErrorCard
        kind="EngineUnreachable"
        message={
          "Thuki's engine isn't running\nSend your message again to restart it."
        }
      />,
    );
    expect(
      screen.getByText("Thuki's engine isn't running"),
    ).toBeInTheDocument();
    expect(
      screen.getByText('Send your message again to restart it.'),
    ).toBeInTheDocument();
  });

  it('pins the exact ollama EngineUnreachable copy', () => {
    render(
      <ErrorCard
        kind="EngineUnreachable"
        message={"Ollama isn't running\nStart Ollama and try again."}
      />,
    );
    expect(screen.getByText("Ollama isn't running")).toBeInTheDocument();
    expect(screen.getByText('Start Ollama and try again.')).toBeInTheDocument();
  });

  it('renders the builtin ModelNotFound copy without a code element', () => {
    const { container } = render(
      <ErrorCard
        kind="ModelNotFound"
        message={'Model not found\nPick or download a model in Settings.'}
      />,
    );
    expect(
      screen.getByText('Pick or download a model in Settings.'),
    ).toBeInTheDocument();
    // No ollama pull command in the builtin copy, so nothing is code-wrapped.
    expect(container.querySelector('code')).toBeNull();
  });

  // EngineStartFailed: fixed human title + the raw backend detail verbatim in a
  // wrapped, scrollable block, plus a Switch model recovery action.
  describe('EngineStartFailed', () => {
    const RAW_DETAIL =
      '0.00.032.387 E llama_model_load: error loading model: illegal split file idx: 1 (file: /Users/logan/Library/Application Support/com.quietnode.thuki/models/blobs/2b0095251d3b1cf9a4ca9d6f8a2793715422f90e9468ca2b3deef766a368a6d9), model must be loaded with the first split';

    it('renders the fixed engine-start-failed title', () => {
      render(<ErrorCard kind="EngineStartFailed" message={RAW_DETAIL} />);
      expect(
        screen.getByText("Thuki's engine couldn't start this model"),
      ).toBeInTheDocument();
    });

    it('renders the raw backend detail verbatim, wrapped and scrollable', () => {
      render(<ErrorCard kind="EngineStartFailed" message={RAW_DETAIL} />);
      const detail = screen.getByText(RAW_DETAIL);
      expect(detail).toBeInTheDocument();
      // The detail block wraps and scrolls so a long blob path never overflows.
      expect(detail.style.whiteSpace).toBe('normal');
      expect(detail.style.overflowWrap).toBe('anywhere');
      expect(detail.style.wordBreak).toBe('break-word');
      expect(detail.style.maxHeight).toBe('84px');
      expect(detail.style.overflow).toBe('auto');
    });

    it('shows the raw detail even when it contains no newline', () => {
      render(
        <ErrorCard kind="EngineStartFailed" message="single line failure" />,
      );
      expect(screen.getByText('single line failure')).toBeInTheDocument();
    });

    it('renders a Switch model button when onSwitchModel is provided', () => {
      render(
        <ErrorCard
          kind="EngineStartFailed"
          message={RAW_DETAIL}
          onSwitchModel={vi.fn()}
        />,
      );
      expect(
        screen.getByRole('button', { name: 'Switch model' }),
      ).toBeInTheDocument();
    });

    it('omits the Switch model button when onSwitchModel is absent', () => {
      render(<ErrorCard kind="EngineStartFailed" message={RAW_DETAIL} />);
      expect(screen.queryByRole('button', { name: 'Switch model' })).toBeNull();
    });

    it('fires onSwitchModel when the Switch model button is clicked', () => {
      const onSwitchModel = vi.fn();
      render(
        <ErrorCard
          kind="EngineStartFailed"
          message={RAW_DETAIL}
          onSwitchModel={onSwitchModel}
        />,
      );
      fireEvent.click(screen.getByRole('button', { name: 'Switch model' }));
      expect(onSwitchModel).toHaveBeenCalledTimes(1);
    });

    it('keeps the red accent bar', () => {
      const { container } = render(
        <ErrorCard kind="EngineStartFailed" message={RAW_DETAIL} />,
      );
      const bar = container.querySelector('[data-error-bar]');
      expect(bar?.getAttribute('data-kind')).toBe('EngineStartFailed');
      expect((bar as HTMLElement | null)?.style.background).toBe(
        'rgb(239, 68, 68)',
      );
    });
  });

  // InsufficientMemory (issue #296): dedicated three-line card with a
  // "Switch model" / "Load anyway" pair, sourced from the machine-readable
  // figures the caller fetches via `estimate_model_fit`.
  describe('InsufficientMemory', () => {
    const INFO = {
      modelName: 'Qwen3.5 9B',
      requiredBytes: 8 * 1024 ** 3,
      availableBytes: 4 * 1024 ** 3,
      canRemember: true,
    };
    const FALLBACK_MESSAGE =
      'This model may not fit in memory\nClose some apps, pick a smaller model, or load it anyway.';

    it('renders the dynamic title with the model name', () => {
      render(
        <ErrorCard
          kind="InsufficientMemory"
          message={FALLBACK_MESSAGE}
          insufficientMemoryInfo={INFO}
        />,
      );
      expect(
        screen.getByText('Qwen3.5 9B may not fit in memory right now.'),
      ).toBeInTheDocument();
    });

    it('renders estimated need and available memory as one-decimal GB', () => {
      render(
        <ErrorCard
          kind="InsufficientMemory"
          message={FALLBACK_MESSAGE}
          insufficientMemoryInfo={INFO}
        />,
      );
      expect(
        screen.getByText(
          'Estimated need: ~8.0 GB. Currently available: ~4.0 GB.',
        ),
      ).toBeInTheDocument();
    });

    it('renders the fixed reboot warning verbatim', () => {
      render(
        <ErrorCard
          kind="InsufficientMemory"
          message={FALLBACK_MESSAGE}
          insufficientMemoryInfo={INFO}
        />,
      );
      expect(
        screen.getByText(
          'To fit this model, your Mac may compress memory, which can slow things down or, in extreme cases, freeze the entire machine and require a reboot.',
        ),
      ).toBeInTheDocument();
    });

    it('applies the amber accent bar in the mild band', () => {
      const { container } = render(
        <ErrorCard
          kind="InsufficientMemory"
          message={FALLBACK_MESSAGE}
          insufficientMemoryInfo={INFO}
        />,
      );
      const bar = container.querySelector('[data-error-bar]');
      expect(bar?.getAttribute('data-kind')).toBe('InsufficientMemory');
      expect((bar as HTMLElement | null)?.style.background).toBe(
        'rgb(245, 158, 11)',
      );
    });

    it('tints the accent bar red in the freeze band so it matches the severity tag', () => {
      const { container } = render(
        <ErrorCard
          kind="InsufficientMemory"
          message={FALLBACK_MESSAGE}
          insufficientMemoryInfo={{ ...INFO, canRemember: false }}
        />,
      );
      const bar = container.querySelector('[data-error-bar]');
      expect((bar as HTMLElement | null)?.style.background).toBe(
        'rgb(248, 113, 113)',
      );
    });

    it('renders the three split actions in the mild band (canRemember true)', () => {
      render(
        <ErrorCard
          kind="InsufficientMemory"
          message={FALLBACK_MESSAGE}
          insufficientMemoryInfo={INFO}
          onLoadAnyway={vi.fn()}
          onSwitchModel={vi.fn()}
        />,
      );
      expect(
        screen.getByRole('button', { name: 'Load once' }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole('button', { name: 'Always allow this model' }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole('button', { name: 'Switch model' }),
      ).toBeInTheDocument();
      // No single "Load anyway" in the mild band; it is split.
      expect(screen.queryByRole('button', { name: 'Load anyway' })).toBeNull();
    });

    it('mild band: "Load once" fires onLoadAnyway(false), "Always allow this model" fires onLoadAnyway(true), Switch fires onSwitchModel', () => {
      const onLoadAnyway = vi.fn();
      const onSwitchModel = vi.fn();
      render(
        <ErrorCard
          kind="InsufficientMemory"
          message={FALLBACK_MESSAGE}
          insufficientMemoryInfo={INFO}
          onLoadAnyway={onLoadAnyway}
          onSwitchModel={onSwitchModel}
        />,
      );
      fireEvent.click(screen.getByRole('button', { name: 'Load once' }));
      expect(onLoadAnyway).toHaveBeenLastCalledWith(false);
      fireEvent.click(
        screen.getByRole('button', { name: 'Always allow this model' }),
      );
      expect(onLoadAnyway).toHaveBeenLastCalledWith(true);
      fireEvent.click(screen.getByRole('button', { name: 'Switch model' }));
      expect(onSwitchModel).toHaveBeenCalledTimes(1);
    });

    it('freeze band (canRemember false): chip, free-vs-needed title, single note, two buttons', () => {
      const onLoadAnyway = vi.fn();
      const onSwitchModel = vi.fn();
      render(
        <ErrorCard
          kind="InsufficientMemory"
          message={FALLBACK_MESSAGE}
          insufficientMemoryInfo={{ ...INFO, canRemember: false }}
          onLoadAnyway={onLoadAnyway}
          onSwitchModel={onSwitchModel}
        />,
      );
      // Severity tag: red tokens, squared corners, and no dot of its own.
      const chip = screen.getByTestId('memory-critical-chip') as HTMLElement;
      expect(chip).toHaveTextContent('Memory critically low');
      expect(chip.style.color).toBe('rgb(248, 113, 113)');
      expect(chip.style.background).toBe('rgba(248, 113, 113, 0.12)');
      expect(chip.style.border).toBe('1px solid rgba(248, 113, 113, 0.35)');
      // Squared-off tag, not a pill.
      expect(chip.style.borderRadius).toBe('5px');
      expect(chip.style.fontSize).toBe('10px');
      expect(chip.style.fontWeight).toBe('700');
      expect(chip.style.letterSpacing).toBe('0.08em');
      expect(chip.style.padding).toBe('3px 8px');
      expect(chip.style.textTransform).toBe('uppercase');
      // Text only: the tag itself is the indicator, so it carries no dot.
      expect(chip.querySelector('span')).toBeNull();
      // Free-vs-needed title, using the card's existing GB rounding.
      expect(
        screen.getByText('Only ~4.0 GB free. Qwen3.5 9B needs ~8.0 GB.'),
      ).toBeInTheDocument();
      // The single severity note.
      expect(screen.getByTestId('memory-freeze-note')).toHaveTextContent(
        'That is far too tight to load on its own. Thuki always asks at this level, because loading can slow your Mac badly or freeze it.',
      );
      // The old fit/estimate/consequence copy is gone in this band.
      expect(
        screen.queryByText('Qwen3.5 9B may not fit in memory right now.'),
      ).toBeNull();
      expect(
        screen.queryByText(
          'Estimated need: ~8.0 GB. Currently available: ~4.0 GB.',
        ),
      ).toBeNull();
      expect(screen.queryByText(INSUFFICIENT_MEMORY_CONSEQUENCE)).toBeNull();
      // Exactly Load anyway + Switch model; never "Always allow this model".
      expect(
        screen.queryByRole('button', { name: 'Always allow this model' }),
      ).toBeNull();
      expect(screen.queryByRole('button', { name: 'Load once' })).toBeNull();
      expect(screen.getAllByRole('button')).toHaveLength(2);
      fireEvent.click(screen.getByRole('button', { name: 'Load anyway' }));
      expect(onLoadAnyway).toHaveBeenCalledWith(false);
      fireEvent.click(screen.getByRole('button', { name: 'Switch model' }));
      expect(onSwitchModel).toHaveBeenCalledTimes(1);
    });

    it('mild band renders no chip and no freeze note', () => {
      render(
        <ErrorCard
          kind="InsufficientMemory"
          message={FALLBACK_MESSAGE}
          insufficientMemoryInfo={INFO}
          onLoadAnyway={vi.fn()}
        />,
      );
      expect(screen.queryByTestId('memory-freeze-note')).toBeNull();
      expect(screen.queryByTestId('memory-critical-chip')).toBeNull();
    });

    it('renders only Switch model when onLoadAnyway is absent (mild band)', () => {
      const onSwitchModel = vi.fn();
      render(
        <ErrorCard
          kind="InsufficientMemory"
          message={FALLBACK_MESSAGE}
          insufficientMemoryInfo={INFO}
          onSwitchModel={onSwitchModel}
        />,
      );
      expect(
        screen.getByRole('button', { name: 'Switch model' }),
      ).toBeInTheDocument();
      expect(screen.queryByRole('button', { name: 'Load once' })).toBeNull();
      expect(
        screen.queryByRole('button', { name: 'Always allow this model' }),
      ).toBeNull();
      fireEvent.click(screen.getByRole('button', { name: 'Switch model' }));
      expect(onSwitchModel).toHaveBeenCalledTimes(1);
    });

    it('renders only the force button when onSwitchModel is absent (freeze band)', () => {
      const onLoadAnyway = vi.fn();
      render(
        <ErrorCard
          kind="InsufficientMemory"
          message={FALLBACK_MESSAGE}
          insufficientMemoryInfo={{ ...INFO, canRemember: false }}
          onLoadAnyway={onLoadAnyway}
        />,
      );
      expect(
        screen.getByRole('button', { name: 'Load anyway' }),
      ).toBeInTheDocument();
      expect(screen.queryByRole('button', { name: 'Switch model' })).toBeNull();
      fireEvent.click(screen.getByRole('button', { name: 'Load anyway' }));
      expect(onLoadAnyway).toHaveBeenCalledWith(false);
    });

    it('omits all action buttons when neither handler is provided', () => {
      render(
        <ErrorCard
          kind="InsufficientMemory"
          message={FALLBACK_MESSAGE}
          insufficientMemoryInfo={INFO}
        />,
      );
      expect(screen.queryByRole('button')).toBeNull();
    });

    it('falls back to the generic message render when insufficientMemoryInfo is absent', () => {
      render(
        <ErrorCard kind="InsufficientMemory" message={FALLBACK_MESSAGE} />,
      );
      expect(
        screen.getByText('This model may not fit in memory'),
      ).toBeInTheDocument();
      expect(
        screen.getByText(
          'Close some apps, pick a smaller model, or load it anyway.',
        ),
      ).toBeInTheDocument();
      expect(
        screen.queryByText('Qwen3.5 9B may not fit in memory right now.'),
      ).toBeNull();
    });
  });
});
