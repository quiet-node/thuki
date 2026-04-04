import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { ImageThumbnails } from '../ImageThumbnails';
import type { ThumbnailItem } from '../ImageThumbnails';

describe('ImageThumbnails', () => {
  const defaultItems: ThumbnailItem[] = [
    { id: 'img-1', src: 'blob:http://localhost/img1' },
    { id: 'img-2', src: 'blob:http://localhost/img2' },
  ];

  it('returns null when items is empty', () => {
    const { container } = render(
      <ImageThumbnails items={[]} onPreview={vi.fn()} />,
    );
    expect(container.innerHTML).toBe('');
  });

  it('renders a list container with correct role and aria-label', () => {
    render(<ImageThumbnails items={defaultItems} onPreview={vi.fn()} />);
    const list = screen.getByRole('list', { name: 'Attached images' });
    expect(list).toBeInTheDocument();
  });

  it('renders one listitem per item', () => {
    render(<ImageThumbnails items={defaultItems} onPreview={vi.fn()} />);
    const items = screen.getAllByRole('listitem');
    expect(items).toHaveLength(2);
  });

  it('renders images with the provided src directly', () => {
    render(<ImageThumbnails items={defaultItems} onPreview={vi.fn()} />);
    const images = screen.getAllByAltText('Attached');
    expect(images[0]).toHaveAttribute('src', 'blob:http://localhost/img1');
    expect(images[1]).toHaveAttribute('src', 'blob:http://localhost/img2');
  });

  it('applies default size (56px) to images', () => {
    render(<ImageThumbnails items={defaultItems} onPreview={vi.fn()} />);
    const images = screen.getAllByAltText('Attached');
    expect(images[0]).toHaveStyle({ width: '56px', height: '56px' });
  });

  it('applies custom size to images', () => {
    render(
      <ImageThumbnails items={defaultItems} onPreview={vi.fn()} size={80} />,
    );
    const images = screen.getAllByAltText('Attached');
    expect(images[0]).toHaveStyle({ width: '80px', height: '80px' });
    expect(images[1]).toHaveStyle({ width: '80px', height: '80px' });
  });

  it('calls onPreview with the correct id when thumbnail is clicked', () => {
    const onPreview = vi.fn();
    render(<ImageThumbnails items={defaultItems} onPreview={onPreview} />);
    const previewButtons = screen.getAllByRole('button', {
      name: 'Preview image',
    });
    fireEvent.click(previewButtons[0]);
    expect(onPreview).toHaveBeenCalledWith('img-1');
    fireEvent.click(previewButtons[1]);
    expect(onPreview).toHaveBeenCalledWith('img-2');
    expect(onPreview).toHaveBeenCalledTimes(2);
  });

  it('does not render remove buttons when onRemove is omitted', () => {
    render(<ImageThumbnails items={defaultItems} onPreview={vi.fn()} />);
    expect(screen.queryByRole('button', { name: 'Remove image' })).toBeNull();
  });

  it('renders remove buttons when onRemove is provided', () => {
    render(
      <ImageThumbnails
        items={defaultItems}
        onPreview={vi.fn()}
        onRemove={vi.fn()}
      />,
    );
    const removeButtons = screen.getAllByRole('button', {
      name: 'Remove image',
    });
    expect(removeButtons).toHaveLength(2);
  });

  it('calls onRemove with the correct id when remove button is clicked', () => {
    const onRemove = vi.fn();
    render(
      <ImageThumbnails
        items={defaultItems}
        onPreview={vi.fn()}
        onRemove={onRemove}
      />,
    );
    const removeButtons = screen.getAllByRole('button', {
      name: 'Remove image',
    });
    fireEvent.click(removeButtons[0]);
    expect(onRemove).toHaveBeenCalledWith('img-1');
    fireEvent.click(removeButtons[1]);
    expect(onRemove).toHaveBeenCalledWith('img-2');
    expect(onRemove).toHaveBeenCalledTimes(2);
  });

  it('renders the close icon SVG inside remove buttons with aria-hidden', () => {
    const { container } = render(
      <ImageThumbnails
        items={[{ id: 'img-1', src: 'blob:http://localhost/img' }]}
        onPreview={vi.fn()}
        onRemove={vi.fn()}
      />,
    );
    const svg = container.querySelector('svg');
    expect(svg).not.toBeNull();
    expect(svg?.getAttribute('aria-hidden')).toBe('true');
  });

  it('sets draggable=false on images', () => {
    render(
      <ImageThumbnails
        items={[{ id: 'img-1', src: 'blob:http://localhost/img' }]}
        onPreview={vi.fn()}
      />,
    );
    const img = screen.getByAltText('Attached');
    expect(img).toHaveAttribute('draggable', 'false');
  });

  it('applies opacity class when item is loading', () => {
    render(
      <ImageThumbnails
        items={[
          { id: 'img-1', src: 'blob:http://localhost/img', loading: true },
        ]}
        onPreview={vi.fn()}
      />,
    );
    const img = screen.getByAltText('Attached');
    expect(img.classList.contains('opacity-50')).toBe(true);
  });

  it('does not apply opacity class when item is not loading', () => {
    render(
      <ImageThumbnails
        items={[
          { id: 'img-1', src: 'blob:http://localhost/img', loading: false },
        ]}
        onPreview={vi.fn()}
      />,
    );
    const img = screen.getByAltText('Attached');
    expect(img.classList.contains('opacity-50')).toBe(false);
  });

  it('renders spinner when item is loading', () => {
    const { container } = render(
      <ImageThumbnails
        items={[
          { id: 'img-1', src: 'blob:http://localhost/img', loading: true },
        ]}
        onPreview={vi.fn()}
      />,
    );
    expect(container.querySelector('.animate-spin')).not.toBeNull();
  });

  it('does not render spinner when item is not loading', () => {
    const { container } = render(
      <ImageThumbnails
        items={[
          { id: 'img-1', src: 'blob:http://localhost/img', loading: false },
        ]}
        onPreview={vi.fn()}
      />,
    );
    expect(container.querySelector('.animate-spin')).toBeNull();
  });
});
