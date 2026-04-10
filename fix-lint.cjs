const fs = require('fs');
let code = fs.readFileSync('src/App.tsx', 'utf8');

code = code.replace(
  "windowX: number | null,\n      windowY: number | null,\n      screenBottomY: number | null,",
  "windowX: number | null,\n      windowY: number | null,\n      _screenBottomY: number | null,"
);

code = code.replace(
  "const MAX_CHAT_WINDOW_HEIGHT = 600 + CONTAINER_VERTICAL_PADDING;",
  "// const MAX_CHAT_WINDOW_HEIGHT = 600 + CONTAINER_VERTICAL_PADDING;"
);

fs.writeFileSync('src/App.tsx', code);
