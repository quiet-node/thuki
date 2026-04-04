import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { ImageThumbnails } from '../ImageThumbnails';

describe('ImageThumbnails', () => {
  const defaultPaths = ['/path/to/image1.png', '/path/to/image2.jpg'];

  it('returns null when imagePaths is empty', () => {
    const { container } = render(
      <ImageThumbnails imagePaths={[]} onPreview={vi.fn()} />,
    );
    expect(container.innerHTML).toBe('');
  });

  it('renders a list container with correct role and aria-label', () => {
    render(<ImageThumbnails imagePaths={defaultPaths} onPreview={vi.fn()} />);
    const list = screen.getByRole('list', { name: 'Attached images' });
    expect(list).toBeInTheDocument();
  });

  it('renders one listitem per image path', () => {
    render(<ImageThumbnails imagePaths={defaultPaths} onPreview={vi.fn()} />);
    const items = screen.getAllByRole('listitem');
    expect(items).toHaveLength(2);
  });

  it('renders images with convertFileSrc-transformed src', () => {
    render(<ImageThumbnails imagePaths={defaultPaths} onPreview={vi.fn()} />);
    const images = screen.getAllByAltText('Attached');
    expect(images[0]).toHaveAttribute(
      'src',
      `asset://localhost/${encodeURIComponent('/path/to/image1.png')}`,
    );
    expect(images[1]).toHaveAttribute(
      'src',
      `asset://localhost/${encodeURIComponent('/path/to/image2.jpg')}`,
    );
  });

  it('applies default size (56px) to images', () => {
    render(<ImageThumbnails imagePaths={defaultPaths} onPreview={vi.fn()} />);
    const images = screen.getAllByAltText('Attached');
    expect(images[0]).toHaveStyle({ width: '56px', height: '56px' });
  });

  it('applies custom size to images', () => {
    render(
      <ImageThumbnails
        imagePaths={defaultPaths}
        onPreview={vi.fn()}
        size={80}
      />,
    );
    const images = screen.getAllByAltText('Attached');
    expect(images[0]).toHaveStyle({ width: '80px', height: '80px' });
    expect(images[1]).toHaveStyle({ width: '80px', height: '80px' });
  });

  it('calls onPreview with the correct path when thumbnail is clicked', () => {
    const onPreview = vi.fn();
    render(<ImageThumbnails imagePaths={defaultPaths} onPreview={onPreview} />);
    const previewButtons = screen.getAllByRole('button', {
      name: 'Preview image',
    });
    fireEvent.click(previewButtons[0]);
    expect(onPreview).toHaveBeenCalledWith('/path/to/image1.png');
    fireEvent.click(previewButtons[1]);
    expect(onPreview).toHaveBeenCalledWith('/path/to/image2.jpg');
    expect(onPreview).toHaveBeenCalledTimes(2);
  });

  it('does not render remove buttons when onRemove is omitted', () => {
    render(<ImageThumbnails imagePaths={defaultPaths} onPreview={vi.fn()} />);
    expect(screen.queryByRole('button', { name: 'Remove image' })).toBeNull();
  });

  it('renders remove buttons when onRemove is provided', () => {
    render(
      <ImageThumbnails
        imagePaths={defaultPaths}
        onPreview={vi.fn()}
        onRemove={vi.fn()}
      />,
    );
    const removeButtons = screen.getAllByRole('button', {
      name: 'Remove image',
    });
    expect(removeButtons).toHaveLength(2);
  });

  it('calls onRemove with the correct path when remove button is clicked', () => {
    const onRemove = vi.fn();
    render(
      <ImageThumbnails
        imagePaths={defaultPaths}
        onPreview={vi.fn()}
        onRemove={onRemove}
      />,
    );
    const removeButtons = screen.getAllByRole('button', {
      name: 'Remove image',
    });
    fireEvent.click(removeButtons[0]);
    expect(onRemove).toHaveBeenCalledWith('/path/to/image1.png');
    fireEvent.click(removeButtons[1]);
    expect(onRemove).toHaveBeenCalledWith('/path/to/image2.jpg');
    expect(onRemove).toHaveBeenCalledTimes(2);
  });

  it('renders the close icon SVG inside remove buttons with aria-hidden', () => {
    const { container } = render(
      <ImageThumbnails
        imagePaths={['/img.png']}
        onPreview={vi.fn()}
        onRemove={vi.fn()}
      />,
    );
    const svg = container.querySelector('svg');
    expect(svg).not.toBeNull();
    expect(svg?.getAttribute('aria-hidden')).toBe('true');
  });

  it('sets draggable=false on images', () => {
    render(<ImageThumbnails imagePaths={['/img.png']} onPreview={vi.fn()} />);
    const img = screen.getByAltText('Attached');
    expect(img).toHaveAttribute('draggable', 'false');
  });
});
