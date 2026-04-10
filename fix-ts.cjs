const fs = require('fs');
let code = fs.readFileSync('src/App.tsx', 'utf8');

code = code.replace(
  "payload.screen_bottom_y ?? null,",
  "// payload.screen_bottom_y ?? null,"
);

fs.writeFileSync('src/App.tsx', code);
