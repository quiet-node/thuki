import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { ImagePreviewModal } from '../ImagePreviewModal';

describe('ImagePreviewModal', () => {
  describe('when imagePath is null', () => {
    it('renders nothing', () => {
      const { container } = render(
        <ImagePreviewModal imagePath={null} onClose={vi.fn()} />,
      );
      expect(container.querySelector('[role="dialog"]')).toBeNull();
    });

    it('does not register keydown listener when closed', () => {
      const onClose = vi.fn();
      render(<ImagePreviewModal imagePath={null} onClose={onClose} />);

      fireEvent.keyDown(window, { key: 'Escape' });
      expect(onClose).not.toHaveBeenCalled();
    });
  });

  describe('when imagePath is set', () => {
    const testPath = '/Users/test/photo.png';

    it('renders a dialog with correct aria attributes', () => {
      render(<ImagePreviewModal imagePath={testPath} onClose={vi.fn()} />);
      const dialog = screen.getByRole('dialog');
      expect(dialog).toBeInTheDocument();
      expect(dialog).toHaveAttribute('aria-label', 'Image preview');
    });

    it('renders the image with converted src', () => {
      render(<ImagePreviewModal imagePath={testPath} onClose={vi.fn()} />);
      const img = screen.getByAltText('Preview');
      expect(img).toBeInTheDocument();
      expect(img.getAttribute('src')).toBe(
        `asset://localhost/${encodeURIComponent(testPath)}`,
      );
    });

    it('renders the close button with aria-label', () => {
      render(<ImagePreviewModal imagePath={testPath} onClose={vi.fn()} />);
      expect(
        screen.getByRole('button', { name: 'Close preview' }),
      ).toBeInTheDocument();
    });

    it('renders the close icon SVG with aria-hidden', () => {
      const { container } = render(
        <ImagePreviewModal imagePath={testPath} onClose={vi.fn()} />,
      );
      const svg = container.querySelector('svg');
      expect(svg).not.toBeNull();
      expect(svg!.getAttribute('aria-hidden')).toBe('true');
    });
  });

  describe('closing interactions', () => {
    const testPath = '/Users/test/photo.png';

    it('calls onClose when clicking the backdrop', () => {
      const onClose = vi.fn();
      render(<ImagePreviewModal imagePath={testPath} onClose={onClose} />);

      const dialog = screen.getByRole('dialog');
      fireEvent.click(dialog);
      expect(onClose).toHaveBeenCalledTimes(1);
    });

    it('calls onClose when clicking the close button (also bubbles to backdrop)', () => {
      const onClose = vi.fn();
      render(<ImagePreviewModal imagePath={testPath} onClose={onClose} />);

      fireEvent.click(screen.getByRole('button', { name: 'Close preview' }));
      // The button's onClick fires onClose, and the event bubbles to the
      // backdrop's onClick which also fires onClose — 2 calls total.
      expect(onClose).toHaveBeenCalledTimes(2);
    });

    it('calls onClose on Escape key press', () => {
      const onClose = vi.fn();
      render(<ImagePreviewModal imagePath={testPath} onClose={onClose} />);

      fireEvent.keyDown(window, { key: 'Escape' });
      expect(onClose).toHaveBeenCalledTimes(1);
    });

    it('does not call onClose on non-Escape key press', () => {
      const onClose = vi.fn();
      render(<ImagePreviewModal imagePath={testPath} onClose={onClose} />);

      fireEvent.keyDown(window, { key: 'Enter' });
      expect(onClose).not.toHaveBeenCalled();
    });

    it('clicking the image does not call onClose (stopPropagation)', () => {
      const onClose = vi.fn();
      render(<ImagePreviewModal imagePath={testPath} onClose={onClose} />);

      const img = screen.getByAltText('Preview');
      fireEvent.click(img);
      expect(onClose).not.toHaveBeenCalled();
    });
  });

  describe('keydown listener lifecycle', () => {
    it('removes keydown listener when imagePath changes to null', () => {
      const onClose = vi.fn();
      const { rerender } = render(
        <ImagePreviewModal imagePath="/test.png" onClose={onClose} />,
      );

      // Escape works while open
      fireEvent.keyDown(window, { key: 'Escape' });
      expect(onClose).toHaveBeenCalledTimes(1);

      // Close modal
      rerender(<ImagePreviewModal imagePath={null} onClose={onClose} />);

      // Escape no longer triggers onClose
      fireEvent.keyDown(window, { key: 'Escape' });
      expect(onClose).toHaveBeenCalledTimes(1);
    });

    it('removes keydown listener on unmount', () => {
      const onClose = vi.fn();
      const { unmount } = render(
        <ImagePreviewModal imagePath="/test.png" onClose={onClose} />,
      );

      unmount();
      fireEvent.keyDown(window, { key: 'Escape' });
      expect(onClose).not.toHaveBeenCalled();
    });
  });
});
