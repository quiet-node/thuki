import { motion, AnimatePresence } from 'framer-motion';
import { convertFileSrc } from '@tauri-apps/api/core';

interface ImageThumbnailsProps {
  /** Absolute file paths of the attached images. */
  imagePaths: string[];
  /** Called with the path when a thumbnail is clicked (opens preview). */
  onPreview: (path: string) => void;
  /** Called with the path when the remove button is clicked. Omit to hide remove buttons. */
  onRemove?: (path: string) => void;
  /** Thumbnail size in pixels. Defaults to 56. */
  size?: number;
}

/**
 * Renders a horizontal row of image thumbnails with optional remove buttons.
 * Used in the ask bar (with remove) and in chat bubbles (without remove).
 */
export function ImageThumbnails({
  imagePaths,
  onPreview,
  onRemove,
  size = 56,
}: ImageThumbnailsProps) {
  if (imagePaths.length === 0) return null;

  return (
    <div
      className="flex gap-2 flex-wrap"
      role="list"
      aria-label="Attached images"
    >
      <AnimatePresence>
        {imagePaths.map((path) => (
          <motion.div
            key={path}
            layout
            initial={{ opacity: 0, scale: 0.8 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.8 }}
            transition={{ type: 'spring', stiffness: 400, damping: 25 }}
            className="relative group"
            role="listitem"
          >
            <button
              type="button"
              onClick={() => onPreview(path)}
              className="block rounded-lg overflow-hidden border border-surface-border hover:border-primary/40 transition-colors cursor-pointer"
              aria-label="Preview image"
            >
              <img
                src={convertFileSrc(path)}
                alt="Attached"
                style={{ width: size, height: size }}
                className="object-cover"
                draggable={false}
              />
            </button>
            {onRemove && (
              <button
                type="button"
                onClick={() => onRemove(path)}
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
