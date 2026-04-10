const fs = require('fs');
let code = fs.readFileSync('src/App.tsx', 'utf8');

// The code sets shouldGrowUp based on screen bottom Y. We want to just make it always true!
const oldShouldGrowUp = `      // would overflow the screen bottom, grow upward instead.
      const shouldGrowUp =
        windowY !== null &&
        screenBottomY !== null &&
        windowY + MAX_CHAT_WINDOW_HEIGHT > screenBottomY;`;

const newShouldGrowUp = `      // User explicitly requested to ALWAYS morph from the bottom and grow upward.
      const shouldGrowUp = true;`;

code = code.replace(oldShouldGrowUp, newShouldGrowUp);
fs.writeFileSync('src/App.tsx', code);
