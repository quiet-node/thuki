import { motion, AnimatePresence } from 'framer-motion';
import { useEffect, useCallback } from 'react';

interface ImagePreviewModalProps {
  /** URL of the image to preview (blob URL or asset URL). Null when closed. */
  imageUrl: string | null;
  /** Called when the modal should close. */
  onClose: () => void;
}

/**
 * Full-screen modal overlay that displays an image at its natural size,
 * scaled to fit the viewport. Closes on backdrop click, close button,
 * or Escape key.
 */
export function ImagePreviewModal({
  imageUrl,
  onClose,
}: ImagePreviewModalProps) {
  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        onClose();
      }
    },
    [onClose],
  );

  useEffect(() => {
    if (!imageUrl) return;
    window.addEventListener('keydown', handleKeyDown, { capture: true });
    return () =>
      window.removeEventListener('keydown', handleKeyDown, { capture: true });
  }, [imageUrl, handleKeyDown]);

  return (
    <AnimatePresence>
      {imageUrl && (
        <motion.div
          key="image-preview-backdrop"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.15 }}
          onClick={onClose}
          className="fixed inset-0 z-[100] flex items-center justify-center bg-black/70 backdrop-blur-sm"
          role="dialog"
          aria-label="Image preview"
        >
          <motion.img
            key="image-preview"
            src={imageUrl}
            alt="Preview"
            initial={{ scale: 0.9, opacity: 0 }}
            animate={{ scale: 1, opacity: 1 }}
            exit={{ scale: 0.9, opacity: 0 }}
            transition={{ type: 'spring', stiffness: 400, damping: 30 }}
            onClick={(e) => e.stopPropagation()}
            className="max-w-[90%] max-h-[90%] object-contain rounded-lg shadow-2xl"
          />
          <button
            type="button"
            onClick={onClose}
            aria-label="Close preview"
            className="absolute top-4 right-4 w-8 h-8 flex items-center justify-center rounded-full bg-white/10 hover:bg-white/20 text-white transition-colors cursor-pointer"
          >
            <svg
              width="14"
              height="14"
              viewBox="0 0 14 14"
              fill="none"
              xmlns="http://www.w3.org/2000/svg"
              aria-hidden="true"
            >
              <path
                d="M1 1L13 13M13 1L1 13"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
              />
            </svg>
          </button>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
