const fs = require('fs');

let content = fs.readFileSync('src/App.tsx', 'utf8');

const oldEffect = `  useEffect(() => {
    /* v8 ignore start -- ResizeObserver + DOM mutations require a real browser */
    const container = morphingContainerNodeRef.current;
    if (!container) return;

    if (!isChatMode || !isHistoryOpen) {
      if (isChatMode && growsUpward) {
        if (!container.style.minHeight) {
          container.style.minHeight = \`\${container.offsetHeight}px\`;
          void container.offsetHeight; // Force layout
        }
        requestAnimationFrame(() => {
          container.style.minHeight = '600px';
        });
      } else {
        container.style.minHeight = '';
      }
      return;
    }

    const dropdown = historyDropdownRef.current;
    if (!dropdown) return;

    const sync = () => {
      container.style.minHeight = \`\${dropdown.offsetTop + dropdown.offsetHeight + 8}px\`;
    };

    sync();
    const ro = new ResizeObserver(sync);
    ro.observe(dropdown);
    return () => ro.disconnect();
    /* v8 ignore stop */
  }, [isChatMode, isHistoryOpen, growsUpward]);`;

const newEffect = `  useEffect(() => {
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

content = content.replace(oldEffect, newEffect);

const oldJSX = `                style={{
                  transition: 'min-height 0.25s cubic-bezier(0.16, 1, 0.3, 1)',
                }}`;
const newJSX = `                style={{
                  /* transition starts off using min-height, but runtime effects can change it */
                  transition: 'min-height 0.25s cubic-bezier(0.16, 1, 0.3, 1)',
                }}`;
content = content.replace(oldJSX, newJSX);

fs.writeFileSync('src/App.tsx', content);
