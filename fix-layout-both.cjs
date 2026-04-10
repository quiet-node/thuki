const fs = require('fs');

let content = fs.readFileSync('src/App.tsx', 'utf8');

const oldCode = `    if (!isChatMode || !isHistoryOpen) {
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
    }`;

const newCode = `    if (!isChatMode || !isHistoryOpen) {
      if (isChatMode) {
        // Morph the ask bar into the full height chatview regardless of growth direction.
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
    }`;

content = content.replace(oldCode, newCode);

fs.writeFileSync('src/App.tsx', content);
