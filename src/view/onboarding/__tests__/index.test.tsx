import { render, screen, act } from '@testing-library/react';
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

  it('renders IntroStep when stage is intro', () => {
    render(<OnboardingView stage="intro" onComplete={vi.fn()} />);
    expect(screen.getByText('Good to know')).toBeInTheDocument();
  });
});
