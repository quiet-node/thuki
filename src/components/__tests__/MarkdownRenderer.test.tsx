import React from 'react';
import { render, screen } from '@testing-library/react';
import { describe, it, expect } from 'vitest';
import { MarkdownRenderer } from '../MarkdownRenderer';

describe('MarkdownRenderer', () => {
  describe('Basic rendering', () => {
    it('renders plain text as paragraph', () => {
      const { container } = render(<MarkdownRenderer content="Hello world" />);
      expect(container.querySelector('p')).toBeTruthy();
      expect(container.textContent).toContain('Hello world');
    });

    it('renders h1 heading with correct element', () => {
      const { container } = render(<MarkdownRenderer content="# Heading One" />);
      expect(container.querySelector('h1')).toBeTruthy();
      expect(container.querySelector('h1')!.textContent).toContain('Heading One');
    });

    it('renders bullet lists', () => {
      const { container } = render(
        <MarkdownRenderer content={'- item one\n- item two\n- item three'} />,
      );
      expect(container.querySelector('ul')).toBeTruthy();
      const items = container.querySelectorAll('li');
      expect(items).toHaveLength(3);
      expect(items[0].textContent).toContain('item one');
    });

    it('renders numbered lists', () => {
      const { container } = render(
        <MarkdownRenderer content={'1. first\n2. second\n3. third'} />,
      );
      expect(container.querySelector('ol')).toBeTruthy();
      const items = container.querySelectorAll('li');
      expect(items).toHaveLength(3);
      expect(items[1].textContent).toContain('second');
    });

    it('renders inline code', () => {
      const { container } = render(
        <MarkdownRenderer content="Use `console.log()` for debugging" />,
      );
      expect(container.querySelector('code')).toBeTruthy();
      expect(container.querySelector('code')!.textContent).toBe('console.log()');
    });

    it('renders fenced code blocks', () => {
      const { container } = render(
        <MarkdownRenderer content={'```js\nconst x = 1;\n```'} />,
      );
      expect(container.querySelector('pre')).toBeTruthy();
      expect(container.querySelector('pre code')).toBeTruthy();
      expect(container.querySelector('pre code')!.textContent).toContain('const x = 1;');
    });

    it('renders links with correct href', () => {
      const { container } = render(
        <MarkdownRenderer content="[Visit site](https://example.com)" />,
      );
      const link = container.querySelector('a');
      expect(link).toBeTruthy();
      expect(link!.getAttribute('href')).toBe('https://example.com');
      expect(link!.textContent).toBe('Visit site');
    });

    it('renders bold text', () => {
      const { container } = render(
        <MarkdownRenderer content="This is **bold** text" />,
      );
      expect(container.querySelector('strong')).toBeTruthy();
      expect(container.querySelector('strong')!.textContent).toBe('bold');
    });

    it('renders italic text', () => {
      const { container } = render(
        <MarkdownRenderer content="This is *italic* text" />,
      );
      expect(container.querySelector('em')).toBeTruthy();
      expect(container.querySelector('em')!.textContent).toBe('italic');
    });

    it('applies custom className', () => {
      const { container } = render(
        <MarkdownRenderer content="text" className="custom-class" />,
      );
      const span = container.querySelector('span');
      expect(span).toBeTruthy();
      expect(span!.classList.contains('custom-class')).toBe(true);
    });

    it('applies markdown-body class by default', () => {
      const { container } = render(<MarkdownRenderer content="text" />);
      const span = container.querySelector('span');
      expect(span).toBeTruthy();
      expect(span!.classList.contains('markdown-body')).toBe(true);
    });
  });

  describe('XSS sanitization', () => {
    it('strips script tags', () => {
      const { container } = render(
        <MarkdownRenderer content={'<script>alert("xss")</script>safe text'} />,
      );
      expect(container.querySelector('script')).toBeNull();
      expect(container.innerHTML).not.toContain('<script');
    });

    it('strips onerror event handlers', () => {
      const { container } = render(
        <MarkdownRenderer content={'<img src="x" onerror="alert(1)" />'} />,
      );
      const img = container.querySelector('img');
      if (img) {
        expect(img.getAttribute('onerror')).toBeNull();
      }
      expect(container.innerHTML).not.toContain('onerror');
    });

    it('strips javascript: protocol in links', () => {
      const { container } = render(
        // eslint-disable-next-line no-script-url
        <MarkdownRenderer content={'[click me](javascript:alert(1))'} />,
      );
      const link = container.querySelector('a');
      if (link) {
        const href = link.getAttribute('href');
        // DOMPurify either removes the href entirely or replaces it — must not be javascript:
        if (href !== null) {
          expect(href).not.toMatch(/javascript:/i);
        }
      }
      expect(container.innerHTML).not.toMatch(/javascript:/i);
    });

    it('strips iframe embeds', () => {
      const { container } = render(
        <MarkdownRenderer content={'<iframe src="https://evil.com"></iframe>'} />,
      );
      expect(container.querySelector('iframe')).toBeNull();
      expect(container.innerHTML).not.toContain('<iframe');
    });

    it('allows safe HTML through', () => {
      const { container } = render(
        <MarkdownRenderer content="**bold** and *italic* and `code`" />,
      );
      expect(container.querySelector('strong')).toBeTruthy();
      expect(container.querySelector('em')).toBeTruthy();
      expect(container.querySelector('code')).toBeTruthy();
    });
  });

  describe('Edge cases', () => {
    it('handles empty string', () => {
      const { container } = render(<MarkdownRenderer content="" />);
      const span = container.querySelector('span');
      expect(span).toBeTruthy();
      expect(span!.innerHTML).toBe('');
    });
  });
});
