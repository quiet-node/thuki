import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, beforeEach } from 'vitest';
import { InlineLink } from '../InlineLink';
import { invoke } from '../../testUtils/mocks/tauri';

describe('InlineLink', () => {
  beforeEach(() => {
    invoke.mockClear();
    invoke.mockResolvedValue(undefined);
  });

  it('renders underlined link text with the URL in a hover title', () => {
    render(<InlineLink url="https://huggingface.co">Hugging Face</InlineLink>);
    const link = screen.getByRole('button', { name: 'Hugging Face' });
    expect(link).toHaveAttribute('title', 'https://huggingface.co');
    expect(link.getAttribute('style')).toContain('underline');
  });

  it('opens the URL in the browser when clicked', () => {
    render(<InlineLink url="https://huggingface.co">link</InlineLink>);
    fireEvent.click(screen.getByRole('button'));
    expect(invoke).toHaveBeenCalledWith('open_url', {
      url: 'https://huggingface.co',
    });
  });

  it('uses an explicit aria-label when provided', () => {
    render(
      <InlineLink
        url="https://x.com/quiet_node"
        ariaLabel="Open Logan's profile on X"
      >
        Logan
      </InlineLink>,
    );
    expect(
      screen.getByRole('button', { name: "Open Logan's profile on X" }),
    ).toBeInTheDocument();
  });

  it('merges per-surface style overrides over the base style', () => {
    render(
      <InlineLink url="https://example.com" style={{ fontWeight: 600 }}>
        x
      </InlineLink>,
    );
    const style = screen.getByRole('button').getAttribute('style') ?? '';
    expect(style).toContain('underline'); // base preserved
    expect(style).toMatch(/font-weight:\s*600/); // override applied
  });
});
