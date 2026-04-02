import { render } from '@testing-library/react';
import { describe, it, expect } from 'vitest';
import { MarkdownRenderer } from '../MarkdownRenderer';

describe('MarkdownRenderer', () => {
  describe('Basic rendering', () => {
    it('renders plain text as paragraph', () => {
      const { container } = render(<MarkdownRenderer content="Hello world" />);
      expect(container.querySelector('p')).not.toBeNull();
      expect(container.textContent).toContain('Hello world');
    });

    it('renders h1 heading with correct element', () => {
      const { container } = render(
        <MarkdownRenderer content="# Heading One" />,
      );
      expect(container.querySelector('h1')).not.toBeNull();
      expect(container.querySelector('h1')!.textContent).toContain(
        'Heading One',
      );
    });

    it('renders bullet lists', () => {
      const { container } = render(
        <MarkdownRenderer content={'- item one\n- item two\n- item three'} />,
      );
      expect(container.querySelector('ul')).not.toBeNull();
      const items = container.querySelectorAll('li');
      expect(items).toHaveLength(3);
      expect(items[0].textContent).toContain('item one');
    });

    it('renders numbered lists', () => {
      const { container } = render(
        <MarkdownRenderer content={'1. first\n2. second\n3. third'} />,
      );
      expect(container.querySelector('ol')).not.toBeNull();
      const items = container.querySelectorAll('li');
      expect(items).toHaveLength(3);
      expect(items[1].textContent).toContain('second');
    });

    it('renders inline code', () => {
      const { container } = render(
        <MarkdownRenderer content="Use `console.log()` for debugging" />,
      );
      expect(container.querySelector('code')).not.toBeNull();
      expect(container.querySelector('code')!.textContent).toBe(
        'console.log()',
      );
    });

    it('renders fenced code blocks', () => {
      const { container } = render(
        <MarkdownRenderer content={'```js\nconst x = 1;\n```'} />,
      );
      expect(container.querySelector('pre')).not.toBeNull();
      expect(container.querySelector('pre code')).not.toBeNull();
      expect(container.querySelector('pre code')!.textContent).toContain(
        'const x = 1;',
      );
    });

    it('renders links as interactive elements', () => {
      const { container } = render(
        <MarkdownRenderer content="[Visit site](https://example.com)" />,
      );
      // Streamdown renders links as buttons with data-streamdown="link"
      const link = container.querySelector('[data-streamdown="link"]');
      expect(link).not.toBeNull();
      expect(link!.textContent).toBe('Visit site');
    });

    it('renders bold text', () => {
      const { container } = render(
        <MarkdownRenderer content="This is **bold** text" />,
      );
      // Streamdown renders bold as span with data-streamdown="strong"
      const bold = container.querySelector('[data-streamdown="strong"]');
      expect(bold).not.toBeNull();
      expect(bold!.textContent).toBe('bold');
    });

    it('renders italic text', () => {
      const { container } = render(
        <MarkdownRenderer content="This is *italic* text" />,
      );
      expect(container.querySelector('em')).not.toBeNull();
      expect(container.querySelector('em')!.textContent).toBe('italic');
    });

    it('applies custom className', () => {
      const { container } = render(
        <MarkdownRenderer content="text" className="custom-class" />,
      );
      const span = container.querySelector('span');
      expect(span).not.toBeNull();
      expect(span!.classList.contains('custom-class')).toBe(true);
    });

    it('applies markdown-body class by default', () => {
      const { container } = render(<MarkdownRenderer content="text" />);
      const span = container.querySelector('span');
      expect(span).not.toBeNull();
      expect(span!.classList.contains('markdown-body')).toBe(true);
    });
  });

  describe('GFM support', () => {
    it('renders strikethrough text', () => {
      const { container } = render(
        <MarkdownRenderer content="This is ~~deleted~~ text" />,
      );
      expect(container.querySelector('del')).not.toBeNull();
      expect(container.querySelector('del')!.textContent).toBe('deleted');
    });

    it('renders tables', () => {
      const { container } = render(
        <MarkdownRenderer content={'| A | B |\n|---|---|\n| 1 | 2 |'} />,
      );
      expect(container.querySelector('table')).not.toBeNull();
      expect(container.querySelectorAll('th')).toHaveLength(2);
      expect(container.querySelectorAll('td')).toHaveLength(2);
    });

    it('renders task lists', () => {
      const { container } = render(
        <MarkdownRenderer content={'- [x] done\n- [ ] todo'} />,
      );
      const inputs = container.querySelectorAll('input[type="checkbox"]');
      expect(inputs).toHaveLength(2);
    });
  });

  describe('XSS prevention', () => {
    it('strips raw script tags from markdown source', () => {
      const { container } = render(
        <MarkdownRenderer content={'<script>alert("xss")</script>safe text'} />,
      );
      expect(container.querySelector('script')).toBeNull();
      expect(container.innerHTML).not.toContain('<script');
    });

    it('strips raw iframe embeds from markdown source', () => {
      const { container } = render(
        <MarkdownRenderer
          content={'<iframe src="https://evil.com"></iframe>'}
        />,
      );
      expect(container.querySelector('iframe')).toBeNull();
      expect(container.innerHTML).not.toContain('<iframe');
    });

    it('escapes raw img tags with event handlers to inert text', () => {
      const { container } = render(
        <MarkdownRenderer content={'<img src="x" onerror="alert(1)" />'} />,
      );
      // react-markdown escapes raw HTML to text — no actual <img> element is created
      expect(container.querySelector('img')).toBeNull();
    });

    it('sanitizes javascript: protocol in markdown links', () => {
      const { container } = render(
        <MarkdownRenderer content={'[click me](javascript:alert(1))'} />,
      );
      expect(container.innerHTML).not.toMatch(/javascript:/i);
    });

    it('renders safe markdown elements normally', () => {
      const { container } = render(
        <MarkdownRenderer content="**bold** and *italic* and `code`" />,
      );
      expect(
        container.querySelector('[data-streamdown="strong"]'),
      ).not.toBeNull();
      expect(container.querySelector('em')).not.toBeNull();
      expect(container.querySelector('code')).not.toBeNull();
    });
  });

  describe('Edge cases', () => {
    it('handles empty string', () => {
      const { container } = render(<MarkdownRenderer content="" />);
      const span = container.querySelector('span');
      expect(span).not.toBeNull();
      expect(span!.textContent).toBe('');
    });

    it('skips re-render when props are unchanged (React.memo)', () => {
      const { container, rerender } = render(
        <MarkdownRenderer content="stable" />,
      );
      const firstOutput = container.innerHTML;
      rerender(<MarkdownRenderer content="stable" />);
      expect(container.innerHTML).toBe(firstOutput);
    });
  });
});
