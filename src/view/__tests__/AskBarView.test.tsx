import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { AskBarView } from '../AskBarView';
import type { AttachedImage } from '../../types/image';

function makeRef(): React.RefObject<HTMLTextAreaElement | null> {
  return { current: null };
}

/** Helper to create an AttachedImage with defaults. */
function makeImage(overrides: Partial<AttachedImage> = {}): AttachedImage {
  return {
    id: overrides.id ?? 'test-id',
    blobUrl: overrides.blobUrl ?? 'blob:http://localhost/test',
    filePath: overrides.filePath ?? '/tmp/img.jpg',
    ...overrides,
  };
}

/** Default image-related props shared across all AskBarView test renders. */
const IMAGE_DEFAULTS = {
  attachedImages: [] as AttachedImage[],
  onImagesAttached: vi.fn(),
  onImageRemove: vi.fn(),
  onImagePreview: vi.fn(),
  onScreenshot: vi.fn(),
};

describe('AskBarView', () => {
  it('renders textarea with placeholder for input bar mode', () => {
    render(
      <AskBarView
        {...IMAGE_DEFAULTS}
        query=""
        setQuery={vi.fn()}
        isChatMode={false}
        isGenerating={false}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
    expect(textarea).not.toBeNull();
  });

  it('renders textarea with chat mode placeholder', () => {
    render(
      <AskBarView
        {...IMAGE_DEFAULTS}
        query=""
        setQuery={vi.fn()}
        isChatMode={true}
        isGenerating={false}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    const textarea = screen.getByPlaceholderText('Reply...');
    expect(textarea).not.toBeNull();
  });

  it('calls setQuery on textarea change', () => {
    const setQuery = vi.fn();
    render(
      <AskBarView
        {...IMAGE_DEFAULTS}
        query=""
        setQuery={setQuery}
        isChatMode={false}
        isGenerating={false}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
    fireEvent.change(textarea, { target: { value: 'hello' } });
    expect(setQuery).toHaveBeenCalledWith('hello');
  });

  it('disables textarea during generation', () => {
    render(
      <AskBarView
        {...IMAGE_DEFAULTS}
        query=""
        setQuery={vi.fn()}
        isChatMode={false}
        isGenerating={true}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
    expect((textarea as HTMLTextAreaElement).disabled).toBe(true);
  });

  it('calls onSubmit on Enter key', () => {
    const onSubmit = vi.fn();
    render(
      <AskBarView
        {...IMAGE_DEFAULTS}
        query="hello"
        setQuery={vi.fn()}
        isChatMode={false}
        isGenerating={false}
        onSubmit={onSubmit}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
    fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it('does not submit on Shift+Enter', () => {
    const onSubmit = vi.fn();
    render(
      <AskBarView
        {...IMAGE_DEFAULTS}
        query="hello"
        setQuery={vi.fn()}
        isChatMode={false}
        isGenerating={false}
        onSubmit={onSubmit}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
    fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: true });
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('calls onSubmit on button click', () => {
    const onSubmit = vi.fn();
    render(
      <AskBarView
        {...IMAGE_DEFAULTS}
        query="hello"
        setQuery={vi.fn()}
        isChatMode={false}
        isGenerating={false}
        onSubmit={onSubmit}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Send message' }));
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it('shows logo at 40px in input bar mode (w-10 h-10 rounded-xl classes)', () => {
    const { container } = render(
      <AskBarView
        {...IMAGE_DEFAULTS}
        query=""
        setQuery={vi.fn()}
        isChatMode={false}
        isGenerating={false}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    const logo = container.querySelector('img[alt="Thuki"]');
    expect(logo).not.toBeNull();
    expect(logo?.classList.contains('w-10')).toBe(true);
    expect(logo?.classList.contains('h-10')).toBe(true);
    expect(logo?.classList.contains('rounded-xl')).toBe(true);
  });

  it('shows logo at 24px in chat mode (w-6 h-6 rounded-lg classes)', () => {
    const { container } = render(
      <AskBarView
        {...IMAGE_DEFAULTS}
        query=""
        setQuery={vi.fn()}
        isChatMode={true}
        isGenerating={false}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    const logo = container.querySelector('img[alt="Thuki"]');
    expect(logo).not.toBeNull();
    expect(logo?.classList.contains('w-6')).toBe(true);
    expect(logo?.classList.contains('h-6')).toBe(true);
    expect(logo?.classList.contains('rounded-lg')).toBe(true);
  });

  it('shows send button with accessible label', () => {
    render(
      <AskBarView
        {...IMAGE_DEFAULTS}
        query=""
        setQuery={vi.fn()}
        isChatMode={false}
        isGenerating={false}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    expect(
      screen.getByRole('button', { name: 'Send message' }),
    ).toBeInTheDocument();
  });

  it('displays selectedText when provided', () => {
    render(
      <AskBarView
        {...IMAGE_DEFAULTS}
        query=""
        setQuery={vi.fn()}
        isChatMode={false}
        isGenerating={false}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
        selectedText="some highlighted text"
      />,
    );
    expect(screen.getByText(/some highlighted text/)).toBeInTheDocument();
  });

  it('hides context area when no selectedText', () => {
    const { container } = render(
      <AskBarView
        {...IMAGE_DEFAULTS}
        query=""
        setQuery={vi.fn()}
        isChatMode={false}
        isGenerating={false}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    expect(container.querySelector('.whitespace-pre-wrap')).toBeNull();
  });

  it('shows stop button with accessible label during generation', () => {
    render(
      <AskBarView
        {...IMAGE_DEFAULTS}
        query=""
        setQuery={vi.fn()}
        isChatMode={true}
        isGenerating={true}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    expect(
      screen.getByRole('button', { name: 'Stop generating' }),
    ).toBeInTheDocument();
  });

  it('calls onCancel when stop button is clicked', () => {
    const onCancel = vi.fn();
    render(
      <AskBarView
        {...IMAGE_DEFAULTS}
        query=""
        setQuery={vi.fn()}
        isChatMode={true}
        isGenerating={true}
        onSubmit={vi.fn()}
        onCancel={onCancel}
        inputRef={makeRef()}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Stop generating' }));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it('applies spinning ring class to stop button', () => {
    render(
      <AskBarView
        {...IMAGE_DEFAULTS}
        query=""
        setQuery={vi.fn()}
        isChatMode={true}
        isGenerating={true}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    const btn = screen.getByRole('button', { name: 'Stop generating' });
    expect(btn.classList.contains('stop-btn-ring')).toBe(true);
  });

  it('does not call onSubmit when stop button is clicked during generation', () => {
    const onSubmit = vi.fn();
    render(
      <AskBarView
        {...IMAGE_DEFAULTS}
        query="hello"
        setQuery={vi.fn()}
        isChatMode={true}
        isGenerating={true}
        onSubmit={onSubmit}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Stop generating' }));
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('displays selectedText with whitespace-pre-wrap class', () => {
    const { container } = render(
      <AskBarView
        {...IMAGE_DEFAULTS}
        query=""
        setQuery={vi.fn()}
        isChatMode={false}
        isGenerating={false}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
        selectedText="context text here"
      />,
    );
    const el = container.querySelector('.whitespace-pre-wrap');
    expect(el).not.toBeNull();
    expect(el?.textContent).toContain('context text here');
  });

  describe('history icon button', () => {
    it('renders history icon button in ask-bar mode when onHistoryOpen is provided', () => {
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
          onHistoryOpen={vi.fn()}
        />,
      );
      expect(
        screen.getByRole('button', { name: /history/i }),
      ).toBeInTheDocument();
    });

    it('does not render history icon button in chat mode', () => {
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query=""
          setQuery={vi.fn()}
          isChatMode={true}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
          onHistoryOpen={vi.fn()}
        />,
      );
      expect(screen.queryByRole('button', { name: /history/i })).toBeNull();
    });

    it('calls onHistoryOpen when history button is clicked', () => {
      const onHistoryOpen = vi.fn();
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
          onHistoryOpen={onHistoryOpen}
        />,
      );
      fireEvent.click(screen.getByRole('button', { name: /history/i }));
      expect(onHistoryOpen).toHaveBeenCalledOnce();
    });
  });

  describe('image attachments', () => {
    it('renders image thumbnails when attachedImages is non-empty', () => {
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          attachedImages={[
            makeImage({ id: 'img-1', blobUrl: 'blob:http://localhost/1' }),
            makeImage({ id: 'img-2', blobUrl: 'blob:http://localhost/2' }),
          ]}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      expect(
        screen.getByRole('list', { name: /attached images/i }),
      ).toBeInTheDocument();
      expect(screen.getAllByRole('listitem')).toHaveLength(2);
    });

    it('does not render thumbnails when attachedImages is empty', () => {
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          attachedImages={[]}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      expect(
        screen.queryByRole('list', { name: /attached images/i }),
      ).toBeNull();
    });

    it('enables submit button when images are attached even without text', () => {
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          attachedImages={[makeImage({ id: 'img-1' })]}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const btn = screen.getByRole('button', { name: 'Send message' });
      expect(btn).not.toBeDisabled();
    });

    it('calls onImagePreview when thumbnail is clicked', () => {
      const onImagePreview = vi.fn();
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          attachedImages={[makeImage({ id: 'img-1' })]}
          onImagePreview={onImagePreview}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      fireEvent.click(screen.getByRole('button', { name: /preview/i }));
      expect(onImagePreview).toHaveBeenCalledWith('img-1');
    });

    it('calls onImageRemove when remove button is clicked', () => {
      const onImageRemove = vi.fn();
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          attachedImages={[makeImage({ id: 'img-1' })]}
          onImageRemove={onImageRemove}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      fireEvent.click(screen.getByRole('button', { name: /remove/i }));
      expect(onImageRemove).toHaveBeenCalledWith('img-1');
    });

    it('applies drag-over styling on dragOver event', () => {
      const { container } = render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const wrapper = container.firstElementChild!;
      fireEvent.dragOver(wrapper, { preventDefault: vi.fn() });
      expect(wrapper.classList.contains('ring-2')).toBe(true);
    });

    it('removes drag-over styling on dragLeave', () => {
      const { container } = render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const wrapper = container.firstElementChild!;
      fireEvent.dragOver(wrapper, { preventDefault: vi.fn() });
      fireEvent.dragLeave(wrapper);
      expect(wrapper.classList.contains('ring-2')).toBe(false);
    });

    it('removes drag-over styling on drop', () => {
      const { container } = render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const wrapper = container.firstElementChild!;
      fireEvent.dragOver(wrapper, { preventDefault: vi.fn() });
      fireEvent.drop(wrapper, {
        preventDefault: vi.fn(),
        dataTransfer: { files: [] },
      });
      expect(wrapper.classList.contains('ring-2')).toBe(false);
    });

    it('calls onImagesAttached on drop with image files', async () => {
      const onImagesAttached = vi.fn();
      const { container } = render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          onImagesAttached={onImagesAttached}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const wrapper = container.firstElementChild!;
      const file = new File(['fake-img-data'], 'photo.png', {
        type: 'image/png',
      });
      fireEvent.drop(wrapper, {
        preventDefault: vi.fn(),
        dataTransfer: { files: [file] },
      });
      await vi.waitFor(() => {
        expect(onImagesAttached).toHaveBeenCalledTimes(1);
      });
    });

    it('ignores non-image files on drop', () => {
      const onImagesAttached = vi.fn();
      const { container } = render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          onImagesAttached={onImagesAttached}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const wrapper = container.firstElementChild!;
      const file = new File(['text'], 'doc.txt', { type: 'text/plain' });
      fireEvent.drop(wrapper, {
        preventDefault: vi.fn(),
        dataTransfer: { files: [file] },
      });
      expect(onImagesAttached).not.toHaveBeenCalled();
    });

    it('ignores drop when already at max images', () => {
      const onImagesAttached = vi.fn();
      const { container } = render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          attachedImages={[
            makeImage({ id: 'a' }),
            makeImage({ id: 'b' }),
            makeImage({ id: 'c' }),
          ]}
          onImagesAttached={onImagesAttached}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const wrapper = container.firstElementChild!;
      const file = new File(['x'], 'img.png', { type: 'image/png' });
      fireEvent.drop(wrapper, {
        preventDefault: vi.fn(),
        dataTransfer: { files: [file] },
      });
      expect(onImagesAttached).not.toHaveBeenCalled();
    });

    it('ignores drop when generating', () => {
      const onImagesAttached = vi.fn();
      const { container } = render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          onImagesAttached={onImagesAttached}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={true}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const wrapper = container.firstElementChild!;
      const file = new File(['x'], 'img.png', { type: 'image/png' });
      fireEvent.drop(wrapper, {
        preventDefault: vi.fn(),
        dataTransfer: { files: [file] },
      });
      expect(onImagesAttached).not.toHaveBeenCalled();
    });

    it('ignores drop with null files', () => {
      const onImagesAttached = vi.fn();
      const { container } = render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          onImagesAttached={onImagesAttached}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const wrapper = container.firstElementChild!;
      fireEvent.drop(wrapper, {
        preventDefault: vi.fn(),
        dataTransfer: { files: null },
      });
      expect(onImagesAttached).not.toHaveBeenCalled();
    });

    it('does not apply drag-over styling when generating', () => {
      const { container } = render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={true}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const wrapper = container.firstElementChild!;
      fireEvent.dragOver(wrapper, { preventDefault: vi.fn() });
      expect(wrapper.classList.contains('ring-2')).toBe(false);
    });

    it('calls onImagesAttached on paste with image', async () => {
      const onImagesAttached = vi.fn();
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          onImagesAttached={onImagesAttached}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      const file = new File(['fake-img'], 'test.png', { type: 'image/png' });
      const clipboardData = {
        items: [{ type: 'image/png', getAsFile: () => file }],
      };
      fireEvent.paste(textarea, { clipboardData });
      // FileReader is async — wait for the next microtask.
      await vi.waitFor(() => {
        expect(onImagesAttached).toHaveBeenCalledTimes(1);
      });
    });

    it('does not call onImagesAttached on paste with text only', () => {
      const onImagesAttached = vi.fn();
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          onImagesAttached={onImagesAttached}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      const clipboardData = {
        items: [{ type: 'text/plain', getAsFile: () => null }],
      };
      fireEvent.paste(textarea, { clipboardData });
      expect(onImagesAttached).not.toHaveBeenCalled();
    });

    it('ignores paste when clipboard has no items', () => {
      const onImagesAttached = vi.fn();
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          onImagesAttached={onImagesAttached}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      fireEvent.paste(textarea, { clipboardData: { items: null } });
      expect(onImagesAttached).not.toHaveBeenCalled();
    });

    it('ignores paste when generating', () => {
      const onImagesAttached = vi.fn();
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          onImagesAttached={onImagesAttached}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={true}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      const file = new File(['x'], 'img.png', { type: 'image/png' });
      const clipboardData = {
        items: [{ type: 'image/png', getAsFile: () => file }],
      };
      fireEvent.paste(textarea, { clipboardData });
      expect(onImagesAttached).not.toHaveBeenCalled();
    });

    it('skips image items where getAsFile returns null', () => {
      const onImagesAttached = vi.fn();
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          onImagesAttached={onImagesAttached}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      const clipboardData = {
        items: [{ type: 'image/png', getAsFile: () => null }],
      };
      fireEvent.paste(textarea, { clipboardData });
      expect(onImagesAttached).not.toHaveBeenCalled();
    });

    it('respects max image limit during paste', async () => {
      const onImagesAttached = vi.fn();
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          attachedImages={[
            makeImage({ id: 'a' }),
            makeImage({ id: 'b' }),
            makeImage({ id: 'c' }),
          ]}
          onImagesAttached={onImagesAttached}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      const file = new File(['x'], 'img.png', { type: 'image/png' });
      const clipboardData = {
        items: [{ type: 'image/png', getAsFile: () => file }],
      };
      fireEvent.paste(textarea, { clipboardData });
      // Should not process since we're already at max.
      expect(onImagesAttached).not.toHaveBeenCalled();
    });
  });

  describe('screenshot button', () => {
    it('renders screenshot button with correct aria-label', () => {
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      expect(
        screen.getByRole('button', { name: 'Take screenshot' }),
      ).not.toBeNull();
    });

    it('calls onScreenshot when clicked', () => {
      const onScreenshot = vi.fn();
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          onScreenshot={onScreenshot}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      fireEvent.click(screen.getByRole('button', { name: 'Take screenshot' }));
      expect(onScreenshot).toHaveBeenCalledOnce();
    });

    it('is disabled while generating', () => {
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query=""
          setQuery={vi.fn()}
          isChatMode={true}
          isGenerating={true}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      expect(
        screen.getByRole('button', { name: 'Take screenshot' }),
      ).toBeDisabled();
    });

    it('is disabled while submit is pending', () => {
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query=""
          setQuery={vi.fn()}
          isChatMode={true}
          isGenerating={false}
          isSubmitPending={true}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      expect(
        screen.getByRole('button', { name: 'Take screenshot' }),
      ).toBeDisabled();
    });

    it('is disabled when max images are already attached', () => {
      const maxImages = [
        makeImage({ id: '1' }),
        makeImage({ id: '2' }),
        makeImage({ id: '3' }),
      ];
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          attachedImages={maxImages}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      expect(
        screen.getByRole('button', { name: 'Take screenshot' }),
      ).toBeDisabled();
    });

    it('is enabled when fewer than max images are attached', () => {
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          attachedImages={[makeImage()]}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      expect(
        screen.getByRole('button', { name: 'Take screenshot' }),
      ).not.toBeDisabled();
    });

    it('renders in chat mode', () => {
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query=""
          setQuery={vi.fn()}
          isChatMode={true}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      expect(
        screen.getByRole('button', { name: 'Take screenshot' }),
      ).not.toBeNull();
    });

    it('has no hover classes when max images are attached', () => {
      const maxImages = [
        makeImage({ id: '1' }),
        makeImage({ id: '2' }),
        makeImage({ id: '3' }),
      ];
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          attachedImages={maxImages}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const btn = screen.getByRole('button', { name: 'Take screenshot' });
      expect(btn.className).not.toContain('hover:text-text-primary');
      expect(btn.className).not.toContain('hover:bg-white/8');
    });

    it('has hover classes when below max images', () => {
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          attachedImages={[makeImage()]}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const btn = screen.getByRole('button', { name: 'Take screenshot' });
      expect(btn.className).toContain('hover:text-text-primary');
      expect(btn.className).toContain('hover:bg-white/8');
    });

    it('shows tooltip explaining limit when camera button is hovered at max images', () => {
      const maxImages = [
        makeImage({ id: '1' }),
        makeImage({ id: '2' }),
        makeImage({ id: '3' }),
      ];
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          attachedImages={maxImages}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const btn = screen.getByRole('button', { name: 'Take screenshot' });
      fireEvent.mouseEnter(btn.parentElement!);
      expect(screen.getByText('Maximum 3 images attached')).toBeInTheDocument();
    });

    it('does not show max-images tooltip when below max images', () => {
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          attachedImages={[makeImage()]}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      expect(
        screen.queryByText('Maximum 3 images attached'),
      ).not.toBeInTheDocument();
    });

    it('shows screenshot tooltip on hover when below max images', () => {
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          attachedImages={[]}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const btn = screen.getByRole('button', { name: 'Take screenshot' });
      fireEvent.mouseEnter(btn.parentElement!);
      expect(screen.getByText('Take a screenshot')).toBeInTheDocument();
    });
  });

  describe('isSubmitPending state', () => {
    it('shows stop button when isSubmitPending is true', () => {
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          attachedImages={[makeImage({ id: 'img-1', filePath: null })]}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          isSubmitPending={true}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const btn = screen.getByRole('button', { name: /stop/i });
      expect(btn).toBeInTheDocument();
      expect(btn.classList.contains('stop-btn-ring')).toBe(true);
    });

    it('disables textarea when isSubmitPending is true', () => {
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          isSubmitPending={true}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      expect((textarea as HTMLTextAreaElement).disabled).toBe(true);
    });

    it('does not apply drag-over styling when isSubmitPending', () => {
      const { container } = render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          isSubmitPending={true}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const wrapper = container.firstElementChild!;
      fireEvent.dragOver(wrapper, { preventDefault: vi.fn() });
      expect(wrapper.classList.contains('ring-2')).toBe(false);
    });

    it('ignores paste when isSubmitPending', () => {
      const onImagesAttached = vi.fn();
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          onImagesAttached={onImagesAttached}
          query=""
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          isSubmitPending={true}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      const file = new File(['x'], 'img.png', { type: 'image/png' });
      const clipboardData = {
        items: [{ type: 'image/png', getAsFile: () => file }],
      };
      fireEvent.paste(textarea, { clipboardData });
      expect(onImagesAttached).not.toHaveBeenCalled();
    });
  });

  describe('command suggestion popover', () => {
    function renderWithQuery(query: string, busy = false) {
      const setQuery = vi.fn();
      const { rerender } = render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query={query}
          setQuery={setQuery}
          isChatMode={false}
          isGenerating={busy}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      return { setQuery, rerender };
    }

    it('shows CommandSuggestion when query starts with "/"', () => {
      renderWithQuery('/');
      expect(
        screen.getByRole('listbox', { name: /command suggestions/i }),
      ).toBeInTheDocument();
    });

    it('shows CommandSuggestion for partial trigger "/sc"', () => {
      renderWithQuery('/sc');
      expect(
        screen.getByRole('listbox', { name: /command suggestions/i }),
      ).toBeInTheDocument();
      expect(screen.getByText('/screen')).toBeInTheDocument();
    });

    it('does not show CommandSuggestion when query does not start with "/"', () => {
      renderWithQuery('hello');
      expect(
        screen.queryByRole('listbox', { name: /command suggestions/i }),
      ).toBeNull();
    });

    it('does not show CommandSuggestion when query has a space after the trigger', () => {
      renderWithQuery('/screen ');
      expect(
        screen.queryByRole('listbox', { name: /command suggestions/i }),
      ).toBeNull();
    });

    it('does not show CommandSuggestion when busy (generating)', () => {
      renderWithQuery('/screen', true);
      expect(
        screen.queryByRole('listbox', { name: /command suggestions/i }),
      ).toBeNull();
    });

    it('Tab key calls setQuery with trigger + space when suggestion is visible', () => {
      const setQuery = vi.fn();
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query="/sc"
          setQuery={setQuery}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      fireEvent.keyDown(textarea, { key: 'Tab' });
      expect(setQuery).toHaveBeenCalledWith('/screen ');
    });

    it('Enter on highlighted row completes the trigger instead of submitting', () => {
      const onSubmit = vi.fn();
      const setQuery = vi.fn();
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query="/sc"
          setQuery={setQuery}
          isChatMode={false}
          isGenerating={false}
          onSubmit={onSubmit}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      expect(setQuery).toHaveBeenCalledWith('/screen ');
      expect(onSubmit).not.toHaveBeenCalled();
    });

    it('Escape dismisses suggestions without changing query', () => {
      const setQuery = vi.fn();
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query="/sc"
          setQuery={setQuery}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      fireEvent.keyDown(textarea, { key: 'Escape' });
      // setQuery is NOT called (query is unchanged)
      expect(setQuery).not.toHaveBeenCalled();
      // Suggestion popover is no longer rendered
      expect(
        screen.queryByRole('listbox', { name: /command suggestions/i }),
      ).toBeNull();
    });

    it('ArrowDown moves highlight to next row', () => {
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query="/"
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      // Initially row 0 is highlighted (only one command, so index stays 0)
      fireEvent.keyDown(textarea, { key: 'ArrowDown' });
      // With one command, ArrowDown wraps back to 0 — row is still highlighted
      const option = screen.getByRole('option');
      expect(option).toHaveAttribute('aria-selected', 'true');
    });

    it('ArrowUp moves highlight to previous row (wraps)', () => {
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query="/"
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      fireEvent.keyDown(textarea, { key: 'ArrowUp' });
      // Single command wraps back to index 0
      const option = screen.getByRole('option');
      expect(option).toHaveAttribute('aria-selected', 'true');
    });

    it('clicking a suggestion row calls setQuery with trigger + space', () => {
      const setQuery = vi.fn();
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query="/"
          setQuery={setQuery}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const option = screen.getByRole('option');
      fireEvent.mouseDown(option);
      expect(setQuery).toHaveBeenCalledWith('/screen ');
    });

    it('Tab does nothing when suggestions are not shown', () => {
      const onSubmit = vi.fn();
      const setQuery = vi.fn();
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query="hello"
          setQuery={setQuery}
          isChatMode={false}
          isGenerating={false}
          onSubmit={onSubmit}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      fireEvent.keyDown(textarea, { key: 'Tab' });
      expect(setQuery).not.toHaveBeenCalled();
      expect(onSubmit).not.toHaveBeenCalled();
    });

    it('Escape does nothing when suggestions are not shown', () => {
      const setQuery = vi.fn();
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query="hello"
          setQuery={setQuery}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      fireEvent.keyDown(textarea, { key: 'Escape' });
      expect(setQuery).not.toHaveBeenCalled();
    });

    it('shows "No commands found" when prefix matches nothing', () => {
      renderWithQuery('/xyz');
      expect(screen.getByText('No commands found')).toBeInTheDocument();
    });

    it('Enter falls through to submit when suggestion list is empty', () => {
      const onSubmit = vi.fn();
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query="/xyz"
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={onSubmit}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      expect(onSubmit).toHaveBeenCalledOnce();
    });

    it('ArrowDown and ArrowUp do nothing when filtered list is empty', () => {
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query="/xyz"
          setQuery={vi.fn()}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      // Should not throw
      fireEvent.keyDown(textarea, { key: 'ArrowDown' });
      fireEvent.keyDown(textarea, { key: 'ArrowUp' });
      // Still shows "No commands found"
      expect(screen.getByText('No commands found')).toBeInTheDocument();
    });

    it('Tab does nothing when filtered list is empty', () => {
      const setQuery = vi.fn();
      render(
        <AskBarView
          {...IMAGE_DEFAULTS}
          query="/xyz"
          setQuery={setQuery}
          isChatMode={false}
          isGenerating={false}
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
          inputRef={makeRef()}
        />,
      );
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      fireEvent.keyDown(textarea, { key: 'Tab' });
      expect(setQuery).not.toHaveBeenCalled();
    });
  });
});
