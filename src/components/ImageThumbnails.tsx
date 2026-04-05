import { motion, AnimatePresence } from 'framer-motion';

interface ThumbnailItem {
  /** Unique key for React list rendering. */
  id: string;
  /** URL to render (blob URL, asset URL, or any valid image src). */
  src: string;
  /** Whether the image is still being processed by the backend. */
  loading?: boolean;
  /**
   * When true, renders a branded screen-capture loading tile instead of an
   * image. Use this when no preview image is available yet (e.g. the /screen
   * capture is in flight and there is no blob URL to show).
   */
  placeholder?: boolean;
}

interface ImageThumbnailsProps {
  /** Images to display as thumbnails. */
  items: ThumbnailItem[];
  /** Called with the item ID when a thumbnail is clicked (opens preview). */
  onPreview: (id: string) => void;
  /** Called with the item ID when the remove button is clicked. Omit to hide remove buttons. */
  onRemove?: (id: string) => void;
  /** Thumbnail size in pixels. Defaults to 56. */
  size?: number;
}

/**
 * Renders a horizontal row of image thumbnails with optional remove buttons.
 * Used in the ask bar (with remove) and in chat bubbles (without remove).
 */
export function ImageThumbnails({
  items,
  onPreview,
  onRemove,
  size = 56,
}: ImageThumbnailsProps) {
  if (items.length === 0) return null;

  return (
    <div
      className="flex gap-2 flex-wrap"
      role="list"
      aria-label="Attached images"
    >
      <AnimatePresence>
        {items.map((item) => (
          <motion.div
            key={item.id}
            layout
            initial={{ opacity: 0, scale: 0.8 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.8 }}
            transition={{ type: 'spring', stiffness: 400, damping: 25 }}
            className="relative group"
            role="listitem"
          >
            {item.placeholder ? (
              /* Screen-capture in-flight: clean minimal tile, no image yet */
              <div
                className="rounded-lg bg-black/15 flex items-center justify-center"
                style={{ width: size, height: size }}
                aria-label="Capturing screen..."
              >
                <div className="w-4 h-4 border-2 border-white/30 border-t-white/80 rounded-full animate-spin" />
              </div>
            ) : (
              <button
                type="button"
                onClick={() => onPreview(item.id)}
                className="block rounded-lg overflow-hidden border border-surface-border hover:border-primary/40 transition-colors cursor-pointer"
                aria-label="Preview image"
              >
                <img
                  src={item.src}
                  alt="Attached"
                  style={{ width: size, height: size }}
                  className={`object-cover ${item.loading ? 'opacity-50' : ''}`}
                  draggable={false}
                />
                {item.loading && (
                  <div className="absolute inset-0 flex items-center justify-center">
                    <div className="w-4 h-4 border-2 border-white/40 border-t-white rounded-full animate-spin" />
                  </div>
                )}
              </button>
            )}
            {onRemove && (
              <button
                type="button"
                onClick={() => onRemove(item.id)}
                aria-label="Remove image"
                className="absolute -top-1.5 -right-1.5 w-5 h-5 flex items-center justify-center rounded-full bg-neutral border border-surface-border text-text-secondary hover:text-red-400 hover:border-red-400/40 transition-colors opacity-0 group-hover:opacity-100 cursor-pointer"
              >
                <svg
                  width="8"
                  height="8"
                  viewBox="0 0 8 8"
                  fill="none"
                  xmlns="http://www.w3.org/2000/svg"
                  aria-hidden="true"
                >
                  <path
                    d="M1 1L7 7M7 1L1 7"
                    stroke="currentColor"
                    strokeWidth="1.5"
                    strokeLinecap="round"
                  />
                </svg>
              </button>
            )}
          </motion.div>
        ))}
      </AnimatePresence>
    </div>
  );
}

export type { ThumbnailItem };
