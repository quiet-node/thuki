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
      screen.getByText("Five quick tips and you're chatting in seconds."),
    ).toBeInTheDocument();
  });

  it('renders all 5 facts', () => {
    render(<IntroStep onComplete={vi.fn()} />);
    expect(screen.getByText('Double-tap')).toBeInTheDocument();
    expect(screen.getByText('to summon')).toBeInTheDocument();
    expect(
      screen.getByText('Select text, then double-tap'),
    ).toBeInTheDocument();
    expect(screen.getByText('Drop in any image')).toBeInTheDocument();
    expect(screen.getByText('for commands')).toBeInTheDocument();
    expect(screen.getByText('Floats above everything')).toBeInTheDocument();
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
});
