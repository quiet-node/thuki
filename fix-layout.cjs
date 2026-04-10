const fs = require('fs');

let content = fs.readFileSync('src/App.tsx', 'utf8');

// Replace useEffect with useLayoutEffect for the morphing container explicit animation
content = content.replace(
  "import { useState, useEffect, useCallback, useRef } from 'react';",
  "import { useState, useEffect, useCallback, useRef, useLayoutEffect } from 'react';"
);

// We need a ref for the prev height
const refDecl = "  const prevHistoryOpenRef = useRef(isHistoryOpen);";
const newRefDecl = "  const prevHistoryOpenRef = useRef(isHistoryOpen);\n  const prevHeightRef = useRef<number>(COLLAPSED_WINDOW_HEIGHT);";
content = content.replace(refDecl, newRefDecl);

const oldEffect = `  useEffect(() => {
    /* v8 ignore start -- ResizeObserver + DOM mutations require a real browser */
    const container = morphingContainerNodeRef.current;
    if (!container) return;

    if (!isChatMode || !isHistoryOpen) {
      if (isChatMode && growsUpward) {
        // Animate height explicitly so content doesn't force an instant jump.
        container.style.transition = 'height 0.25s cubic-bezier(0.16, 1, 0.3, 1)';
        container.style.height = \`\${container.offsetHeight}px\`;
        void container.offsetHeight; // Force layout
        
        requestAnimationFrame(() => {
          container.style.height = '600px';
        });
      } else {
        // Reset to auto sizing and min-height transition for history panel.
        container.style.transition = 'min-height 0.25s cubic-bezier(0.16, 1, 0.3, 1)';
        container.style.height = '';
        container.style.minHeight = '';
      }
      return;
    }

    const dropdown = historyDropdownRef.current;
    if (!dropdown) return;

    container.style.transition = 'min-height 0.25s cubic-bezier(0.16, 1, 0.3, 1)';
    container.style.height = ''; // Let history panel dictate it via minHeight

    const sync = () => {
      container.style.minHeight = \`\${dropdown.offsetTop + dropdown.offsetHeight + 8}px\`;
    };

    sync();
    const ro = new ResizeObserver(sync);
    ro.observe(dropdown);
    return () => ro.disconnect();
    /* v8 ignore stop */
  }, [isChatMode, isHistoryOpen, growsUpward]);`;

const newEffect = `  useLayoutEffect(() => {
    /* v8 ignore start -- ResizeObserver + DOM mutations require a real browser */
    const container = morphingContainerNodeRef.current;
    if (!container) return;

    // Track the height when we are NOT in chat mode natively.
    if (!isChatMode) {
      const h = container.offsetHeight;
      // offsetHeight might read 0 if hidden, so default to collapsed
      prevHeightRef.current = h > 0 ? h : COLLAPSED_WINDOW_HEIGHT;
    }

    if (!isChatMode || !isHistoryOpen) {
      if (isChatMode && growsUpward) {
        // We know we are growing to the max height (600px).
        // Halting paint and forcing the DOM to the PREVIOUS height first
        // so we avoid the sudden layout flash/jump.
        container.style.transition = 'none';
        container.style.height = \`\${prevHeightRef.current}px\`;
        void container.offsetHeight; // Force layout calculation step
        
        requestAnimationFrame(() => {
          container.style.transition = 'height 0.25s cubic-bezier(0.16, 1, 0.3, 1)';
          container.style.height = '600px';
        });
      } else {
        // Safe reset state for everything else
        container.style.transition = 'min-height 0.25s cubic-bezier(0.16, 1, 0.3, 1)';
        container.style.height = '';
        container.style.minHeight = '';
      }
      return;
    }

    const dropdown = historyDropdownRef.current;
    if (!dropdown) return;

    container.style.transition = 'min-height 0.25s cubic-bezier(0.16, 1, 0.3, 1)';
    container.style.height = ''; // Let history panel dictate it via minHeight

    const sync = () => {
      container.style.minHeight = \`\${dropdown.offsetTop + dropdown.offsetHeight + 8}px\`;
    };

    sync();
    const ro = new ResizeObserver(sync);
    ro.observe(dropdown);
    return () => ro.disconnect();
    /* v8 ignore stop */
  }, [isChatMode, isHistoryOpen, growsUpward]);`;

content = content.replace(oldEffect, newEffect);

fs.writeFileSync('src/App.tsx', content);
