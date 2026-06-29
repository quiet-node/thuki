import { fireEvent, render, screen } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { ErrorCard } from '../ErrorCard';

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
});
