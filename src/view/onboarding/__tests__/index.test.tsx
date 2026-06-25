import { render, screen, act, fireEvent } from '@testing-library/react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { OnboardingView } from '../index';
import { invoke } from '../../../testUtils/mocks/tauri';

describe('OnboardingView (orchestrator)', () => {
  beforeEach(() => {
    invoke.mockClear();
    invoke.mockResolvedValue(undefined);
  });

  it('renders PermissionsStep when stage is permissions', async () => {
    render(<OnboardingView stage="permissions" onComplete={vi.fn()} />);
    await act(async () => {});
    expect(screen.getByText("Let's get Thuki set up")).toBeInTheDocument();
  });

  it('renders the roadmap SubscribeStep first when stage is intro', () => {
    render(<OnboardingView stage="intro" onComplete={vi.fn()} />);
    expect(screen.getByText('Where Thuki is headed')).toBeInTheDocument();
    expect(screen.queryByText("You're all set")).toBeNull();
  });

  it('covers the panel, then advances to the tips card on continue', async () => {
    render(<OnboardingView stage="intro" onComplete={vi.fn()} />);
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /maybe later/i }));
    });
    // The panel is hidden before the resize so the swap matches the cross-fade
    // of the backend-driven transitions instead of a visible window jump.
    expect(invoke).toHaveBeenCalledWith('set_overlay_alpha', {
      alpha: 0,
      durationMs: 0,
    });
    expect(screen.getByText("You're all set")).toBeInTheDocument();
  });

  it('renders ModelCheckStep when stage is model_check', async () => {
    render(<OnboardingView stage="model_check" onComplete={vi.fn()} />);
    await act(async () => {});
    expect(screen.getByText('Set up your local AI')).toBeInTheDocument();
  });

  it('renders BuiltinAnnouncementStep when stage is builtin_announcement', () => {
    render(
      <OnboardingView stage="builtin_announcement" onComplete={vi.fn()} />,
    );
    expect(screen.getByText('Local AI, now built in')).toBeInTheDocument();
  });
});
