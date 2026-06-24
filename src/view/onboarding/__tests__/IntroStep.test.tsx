import { render, screen, fireEvent, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { IntroStep } from '../IntroStep';
import { invoke } from '../../../testUtils/mocks/tauri';

describe('IntroStep', () => {
  beforeEach(() => {
    invoke.mockClear();
  });

  it('renders the title', () => {
    render(<IntroStep onComplete={vi.fn()} />);
    expect(screen.getByText("You're all set")).toBeInTheDocument();
  });

  it('renders the subtitle', () => {
    render(<IntroStep onComplete={vi.fn()} />);
    expect(
      screen.getByText("A few quick tips and you're chatting in seconds."),
    ).toBeInTheDocument();
  });

  it('renders all 6 facts', () => {
    render(<IntroStep onComplete={vi.fn()} />);
    expect(screen.getByText('Double-tap')).toBeInTheDocument();
    expect(screen.getByText('to summon')).toBeInTheDocument();
    expect(
      screen.getByText('Select text, then double-tap'),
    ).toBeInTheDocument();
    expect(screen.getByText('Drop in any image')).toBeInTheDocument();
    expect(screen.getByText('for commands')).toBeInTheDocument();
    expect(
      screen.getByText('Run any open-source AI model'),
    ).toBeInTheDocument();
    expect(screen.getByText('Floats above everything')).toBeInTheDocument();
  });

  it('describes the in-app model library on the Run any open-source AI model fact', () => {
    render(<IntroStep onComplete={vi.fn()} />);
    expect(
      screen.getByText(
        'Pick from thousands, download in a click, swap whenever in Settings, with a fit check for your Mac.',
      ),
    ).toBeInTheDocument();
  });

  it('renders generic slash command guidance', () => {
    render(<IntroStep onComplete={vi.fn()} />);
    expect(screen.getByText('/')).toBeInTheDocument();
    expect(
      screen.getByText(
        'Open the slash menu for built-in tools and writing shortcuts right from the ask bar.',
      ),
    ).toBeInTheDocument();
  });

  it('renders the Get Started button', () => {
    render(<IntroStep onComplete={vi.fn()} />);
    expect(
      screen.getByRole('button', { name: /get started/i }),
    ).toBeInTheDocument();
  });

  it('renders the footer note', () => {
    render(<IntroStep onComplete={vi.fn()} />);
    expect(screen.getByText(/private by default/i)).toBeInTheDocument();
  });

  it('renders the ambient download strip inside the card when a status is supplied', () => {
    render(
      <IntroStep
        onComplete={vi.fn()}
        downloadStatus={{
          kind: 'downloading',
          modelName: 'Qwen3.5 9B',
          percent: 15,
          etaSeconds: 180,
          onPause: vi.fn(),
        }}
      />,
    );
    expect(screen.getByTestId('download-status-strip')).toBeInTheDocument();
    expect(screen.getByText('Downloading Qwen3.5 9B')).toBeInTheDocument();
  });

  it('points the ready strip at Get Started, not the (absent) ask bar', () => {
    render(
      <IntroStep
        onComplete={vi.fn()}
        downloadStatus={{ kind: 'ready', modelName: 'gpt-oss 20B' }}
      />,
    );
    expect(
      screen.getByText('gpt-oss 20B ready. Hit Get Started to start chatting!'),
    ).toBeInTheDocument();
  });

  it('renders no download strip when no status is supplied', () => {
    render(<IntroStep onComplete={vi.fn()} />);
    expect(
      screen.queryByTestId('download-status-strip'),
    ).not.toBeInTheDocument();
  });

  it('calls finish_onboarding and onComplete when Get Started is clicked', async () => {
    const onComplete = vi.fn();
    invoke.mockResolvedValue(undefined);
    render(<IntroStep onComplete={onComplete} />);

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /get started/i }));
    });

    expect(invoke).toHaveBeenCalledWith('finish_onboarding');
    expect(onComplete).toHaveBeenCalledTimes(1);
  });

  it('fades the overlay panel back in after the ask bar paints', async () => {
    // finish_onboarding hides the panel (alpha 0) and resizes it under cover;
    // the fade back to alpha 1 is deferred two animation frames past the swap so
    // the first visible frame is the ask bar, not this card. Run both frames
    // synchronously to assert the fade-in fires with the expected arguments.
    const raf = vi
      .spyOn(globalThis, 'requestAnimationFrame')
      .mockImplementation((cb: FrameRequestCallback) => {
        cb(0);
        return 0;
      });
    const onComplete = vi.fn();
    invoke.mockResolvedValue(undefined);
    render(<IntroStep onComplete={onComplete} />);

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /get started/i }));
    });

    expect(invoke).toHaveBeenCalledWith('set_overlay_alpha', {
      alpha: 1,
      durationMs: 150,
    });
    raf.mockRestore();
  });
});
