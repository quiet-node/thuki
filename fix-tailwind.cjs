const fs = require('fs');

let content = fs.readFileSync('src/App.tsx', 'utf8');

content = content.replace(
  "isChatMode\n                    ? `rounded-lg shadow-chat ${growsUpward ? 'min-h-[600px]' : ''}`\n                    : 'rounded-2xl shadow-bar'",
  "isChatMode\n                    ? `rounded-lg shadow-chat`\n                    : 'rounded-2xl shadow-bar'"
);

fs.writeFileSync('src/App.tsx', content);
