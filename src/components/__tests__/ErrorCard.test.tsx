import { render, screen } from '@testing-library/react';
import { describe, it, expect } from 'vitest';
import { ErrorCard } from '../ErrorCard';

describe('ErrorCard', () => {
  it('renders the title (first line of message)', () => {
    render(
      <ErrorCard
        kind="NotRunning"
        message={"Ollama isn't running\nStart Ollama and try again."}
      />,
    );
    expect(screen.getByText("Ollama isn't running")).toBeInTheDocument();
  });

  it('renders the subtitle (second line of message)', () => {
    render(
      <ErrorCard
        kind="NotRunning"
        message={"Ollama isn't running\nStart Ollama and try again."}
      />,
    );
    expect(screen.getByText('Start Ollama and try again.')).toBeInTheDocument();
  });

  it('renders only title when message has no newline', () => {
    render(<ErrorCard kind="Other" message="Something went wrong" />);
    expect(screen.getByText('Something went wrong')).toBeInTheDocument();
  });

  it('applies red accent bar for NotRunning', () => {
    const { container } = render(
      <ErrorCard
        kind="NotRunning"
        message={"Ollama isn't running\nStart Ollama."}
      />,
    );
    const bar = container.querySelector('[data-error-bar]');
    expect(bar).not.toBeNull();
    expect(bar?.getAttribute('data-kind')).toBe('NotRunning');
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
});
