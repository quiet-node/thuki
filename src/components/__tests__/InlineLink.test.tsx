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

  it('renders subtle variant as plain text, restoring the link look on hover', () => {
    render(
      <InlineLink url="https://huggingface.co" subtle>
        Model
      </InlineLink>,
    );
    const link = screen.getByRole('button');
    // At rest: plain heading text, no link colour or underline.
    let style = link.getAttribute('style') ?? '';
    expect(style).toContain('text-decoration: none');
    expect(style).toContain('var(--t1)');
    expect(style).not.toContain('underline');
    expect(link).toHaveAttribute('title', 'https://huggingface.co');

    // On hover: accent colour and underline return.
    fireEvent.mouseEnter(link);
    style = link.getAttribute('style') ?? '';
    expect(style).toContain('underline');
    expect(style).toContain('rgb(255, 184, 146)'); // #ffb892

    // On leave: back to plain text.
    fireEvent.mouseLeave(link);
    style = link.getAttribute('style') ?? '';
    expect(style).toContain('text-decoration: none');
    expect(style).not.toContain('underline');
  });

  it('applies subtleColor to the rest state only, leaving the hover accent intact', () => {
    render(
      <InlineLink url="https://huggingface.co" subtle subtleColor="var(--t2)">
        file.gguf
      </InlineLink>,
    );
    const link = screen.getByRole('button');
    // At rest: the secondary colour override, not the default primary token.
    let style = link.getAttribute('style') ?? '';
    expect(style).toContain('var(--t2)');
    expect(style).not.toContain('var(--t1)');

    // On hover: the accent colour still returns (the override is rest-only).
    fireEvent.mouseEnter(link);
    style = link.getAttribute('style') ?? '';
    expect(style).toContain('rgb(255, 184, 146)'); // #ffb892
    expect(style).not.toContain('var(--t2)');
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
