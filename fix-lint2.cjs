const fs = require('fs');
let code = fs.readFileSync('src/App.tsx', 'utf8');

code = code.replace(
  "_screenBottomY: number | null,",
  ""
);

fs.writeFileSync('src/App.tsx', code);
