import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { SubscribeStep } from '../SubscribeStep';
import { invoke } from '../../../testUtils/mocks/tauri';

describe('SubscribeStep', () => {
  beforeEach(() => {
    invoke.mockClear();
    invoke.mockResolvedValue(undefined);
  });

  it('renders the headline and subtitle', () => {
    render(<SubscribeStep onContinue={vi.fn()} />);
    expect(screen.getByText('Where Thuki is headed')).toBeInTheDocument();
    expect(screen.getByText("A preview of what's coming.")).toBeInTheDocument();
  });

  it('renders all four roadmap items with their descriptions', () => {
    render(<SubscribeStep onContinue={vi.fn()} />);
    expect(screen.getByText('Connect your tools')).toBeInTheDocument();
    expect(
      screen.getByText('Gmail, Slack, Discord, Calendar, and more.'),
    ).toBeInTheDocument();
    expect(screen.getByText('Type with your voice')).toBeInTheDocument();
    expect(
      screen.getByText('Press a key, speak, and get clean text in any app.'),
    ).toBeInTheDocument();
    expect(screen.getByText('Notes from any meeting')).toBeInTheDocument();
    expect(
      screen.getByText('Live transcripts and summaries of any meeting.'),
    ).toBeInTheDocument();
    expect(screen.getByText('Automate the routine')).toBeInTheDocument();
    expect(
      screen.getByText('Teach Thuki multi-step tasks and run them on a word.'),
    ).toBeInTheDocument();
  });

  it('states the free and local guarantee under the roadmap', () => {
    render(<SubscribeStep onContinue={vi.fn()} />);
    expect(
      screen.getByText('All free. All local. Nothing ever leaves your Mac.'),
    ).toBeInTheDocument();
  });

  it('renders the founder note with the inline Logan link', () => {
    render(<SubscribeStep onContinue={vi.fn()} />);
    expect(
      screen.getByText(/founder of Thuki/i, { exact: false }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/I'll personally reach out, I'd love to talk!/i, {
        exact: false,
      }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: /open logan's profile on x/i }),
    ).toBeInTheDocument();
  });

  it("opens Logan's X profile in the browser when the link is clicked", () => {
    render(<SubscribeStep onContinue={vi.fn()} />);
    fireEvent.click(
      screen.getByRole('button', { name: /open logan's profile on x/i }),
    );
    expect(invoke).toHaveBeenCalledWith('open_url', {
      url: 'https://x.com/quiet_node',
    });
  });

  it('shows an error and does not advance when subscribing with an empty email', () => {
    const onContinue = vi.fn();
    render(<SubscribeStep onContinue={onContinue} />);

    fireEvent.click(
      screen.getByRole('button', { name: /help shape what's next for thuki/i }),
    );

    expect(
      screen.getByText(/enter a valid email address/i),
    ).toBeInTheDocument();
    expect(screen.getByLabelText('Email address')).toHaveAttribute(
      'aria-invalid',
      'true',
    );
    expect(onContinue).not.toHaveBeenCalled();
  });

  it('shows an error and does not advance when the email is malformed', () => {
    const onContinue = vi.fn();
    render(<SubscribeStep onContinue={onContinue} />);

    fireEvent.change(screen.getByLabelText('Email address'), {
      target: { value: 'not-an-email' },
    });
    fireEvent.click(
      screen.getByRole('button', { name: /help shape what's next for thuki/i }),
    );

    expect(
      screen.getByText(/enter a valid email address/i),
    ).toBeInTheDocument();
    expect(onContinue).not.toHaveBeenCalled();
  });

  it('advances when subscribing with a valid email', () => {
    const onContinue = vi.fn();
    render(<SubscribeStep onContinue={onContinue} />);

    fireEvent.change(screen.getByLabelText('Email address'), {
      target: { value: '  founder@thuki.app  ' },
    });
    fireEvent.click(
      screen.getByRole('button', { name: /help shape what's next for thuki/i }),
    );

    expect(onContinue).toHaveBeenCalledTimes(1);
    expect(
      screen.queryByText(/enter a valid email address/i),
    ).not.toBeInTheDocument();
  });

  it('clears the error as soon as the user edits the email', () => {
    render(<SubscribeStep onContinue={vi.fn()} />);

    fireEvent.click(
      screen.getByRole('button', { name: /help shape what's next for thuki/i }),
    );
    expect(
      screen.getByText(/enter a valid email address/i),
    ).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('Email address'), {
      target: { value: 'a' },
    });
    expect(
      screen.queryByText(/enter a valid email address/i),
    ).not.toBeInTheDocument();
  });

  it('advances when Maybe later is clicked', () => {
    const onContinue = vi.fn();
    render(<SubscribeStep onContinue={onContinue} />);

    fireEvent.click(screen.getByRole('button', { name: /maybe later/i }));

    expect(onContinue).toHaveBeenCalledTimes(1);
  });

  it('signals focus with an accent border and clears it on blur', () => {
    render(<SubscribeStep onContinue={vi.fn()} />);
    const input = screen.getByLabelText('Email address');

    // The accent border colour (…141, 92…) is unique to the focused state, so
    // its presence/absence proves the default focus ring was replaced.
    fireEvent.focus(input);
    expect(input.getAttribute('style')).toContain('141');

    fireEvent.blur(input);
    expect(input.getAttribute('style')).not.toContain('141');
  });

  it('renders the ambient download strip when a status is supplied', () => {
    render(
      <SubscribeStep
        onContinue={vi.fn()}
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

  it('renders no download strip when no status is supplied', () => {
    render(<SubscribeStep onContinue={vi.fn()} />);
    expect(
      screen.queryByTestId('download-status-strip'),
    ).not.toBeInTheDocument();
  });

  it('keeps the ready line but fits it to this screen, not "Get Started"', () => {
    render(
      <SubscribeStep
        onContinue={vi.fn()}
        downloadStatus={{ kind: 'ready', modelName: 'gpt-oss 20B' }}
      />,
    );
    expect(screen.getByTestId('download-status-strip')).toBeInTheDocument();
    expect(
      screen.getByText("gpt-oss 20B ready. You're good to go!"),
    ).toBeInTheDocument();
    expect(screen.queryByText(/Hit Get Started/i)).not.toBeInTheDocument();
  });
});
