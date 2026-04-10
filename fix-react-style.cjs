const fs = require('fs');
let code = fs.readFileSync('src/App.tsx', 'utf8');

const oldStyle = `                style={{
                  /* transition starts off using min-height, but runtime effects can change it */
                  transition: 'min-height 0.25s cubic-bezier(0.16, 1, 0.3, 1)',
                }}`;

const newStyle = `                style={{
                  transition: 'height 0.25s cubic-bezier(0.16, 1, 0.3, 1), min-height 0.25s cubic-bezier(0.16, 1, 0.3, 1)',
                  ...(isChatMode && !isHistoryOpen ? { height: '600px' } : {})
                }}`;

code = code.replace(oldStyle, newStyle);
fs.writeFileSync('src/App.tsx', code);
