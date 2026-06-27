import { render, screen, fireEvent, act } from '@testing-library/react';
import { describe, it, expect, beforeEach } from 'vitest';
import { BuiltinAnnouncementStep } from '../BuiltinAnnouncementStep';
import { invoke } from '../../../testUtils/mocks/tauri';

describe('BuiltinAnnouncementStep', () => {
  beforeEach(() => {
    invoke.mockClear();
    invoke.mockResolvedValue(undefined);
  });

  it('renders the version pill, title and subtitle with the blog link', () => {
    render(<BuiltinAnnouncementStep />);
    expect(screen.getByText('NEW')).toBeInTheDocument();
    expect(screen.getByText('Local AI, now built in')).toBeInTheDocument();
    // The subtitle text is split by the embedded "v0.15" blog link.
    expect(
      screen.getByText(/ships its own inference engine/),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', {
        name: 'Read about the v0.15 built-in engine',
      }),
    ).toBeInTheDocument();
  });

  it('opens the blog post from the subtitle v0.15 link', () => {
    render(<BuiltinAnnouncementStep />);
    fireEvent.click(
      screen.getByRole('button', {
        name: 'Read about the v0.15 built-in engine',
      }),
    );
    expect(invoke).toHaveBeenCalledWith('open_url', {
      url: 'https://www.thuki.app/blog/thuki-built-in-local-ai-engine',
    });
  });

  it('opens Hugging Face from the model-freedom link', () => {
    render(<BuiltinAnnouncementStep />);
    fireEvent.click(screen.getByRole('button', { name: 'Open Hugging Face' }));
    expect(invoke).toHaveBeenCalledWith('open_url', {
      url: 'https://huggingface.co',
    });
  });

  it('renders the three benefit points', () => {
    render(<BuiltinAnnouncementStep />);
    expect(
      screen.getByText('One app, nothing else to manage'),
    ).toBeInTheDocument();
    expect(
      screen.getByText('No more background Ollama, Thuki runs models itself.'),
    ).toBeInTheDocument();
    expect(screen.getByText('Total AI model freedom')).toBeInTheDocument();
    expect(screen.getByText('Hugging Face')).toBeInTheDocument();
    expect(screen.getByText(/no terminal needed/)).toBeInTheDocument();
    expect(
      screen.getByText('Private, exactly like before'),
    ).toBeInTheDocument();
    expect(
      screen.getByText('Every model runs locally. Nothing leaves your Mac.'),
    ).toBeInTheDocument();
  });

  it('renders the switch-engines line and footer', () => {
    render(<BuiltinAnnouncementStep />);
    expect(
      screen.getByText(
        'Either way, you can switch engines anytime in Settings.',
      ),
    ).toBeInTheDocument();
    expect(screen.getByText(/Added in v0.15/)).toBeInTheDocument();
    expect(
      screen.getByRole('button', {
        name: 'Learn more about the built-in engine',
      }),
    ).toBeInTheDocument();
  });

  it('switches to the built-in provider then advances when Try Built-in is clicked', async () => {
    render(<BuiltinAnnouncementStep />);

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: 'Try Built-in Engine' }),
      );
    });

    expect(invoke).toHaveBeenCalledWith('set_active_provider', {
      providerId: 'builtin',
    });
    expect(invoke).toHaveBeenCalledWith('advance_past_builtin_announcement');
  });

  it('stays on the announcement when the provider switch fails', async () => {
    invoke.mockImplementation((cmd: string) =>
      cmd === 'set_active_provider'
        ? Promise.reject(new Error('write failed'))
        : Promise.resolve(undefined),
    );

    render(<BuiltinAnnouncementStep />);

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: 'Try Built-in Engine' }),
      );
    });

    expect(invoke).toHaveBeenCalledWith('set_active_provider', {
      providerId: 'builtin',
    });
    expect(invoke).not.toHaveBeenCalledWith(
      'advance_past_builtin_announcement',
    );
  });

  it('advances without switching providers when Keep using Ollama is clicked', async () => {
    render(<BuiltinAnnouncementStep />);

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: 'Keep using Ollama' }),
      );
    });

    expect(invoke).toHaveBeenCalledWith('advance_past_builtin_announcement');
    expect(invoke).not.toHaveBeenCalledWith('set_active_provider', {
      providerId: 'builtin',
    });
  });

  it('opens the learn-more URL when the footer link is clicked', () => {
    render(<BuiltinAnnouncementStep />);

    fireEvent.click(
      screen.getByRole('button', {
        name: 'Learn more about the built-in engine',
      }),
    );

    expect(invoke).toHaveBeenCalledWith('open_url', {
      url: 'https://www.thuki.app/blog/thuki-built-in-local-ai-engine',
    });
  });

  it('lifts the secondary link colour on hover and restores it on leave', () => {
    render(<BuiltinAnnouncementStep />);
    const link = screen.getByRole('button', { name: 'Keep using Ollama' });

    expect(link).toHaveStyle({ color: 'rgba(255,255,255,0.4)' });
    fireEvent.mouseEnter(link);
    expect(link).toHaveStyle({ color: 'rgba(255,255,255,0.7)' });
    fireEvent.mouseLeave(link);
    expect(link).toHaveStyle({ color: 'rgba(255,255,255,0.4)' });
  });
});
