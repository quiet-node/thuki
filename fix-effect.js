const fs = require('fs');

let content = fs.readFileSync('src/App.tsx', 'utf8');

content = content.replace(
  "if (isChatMode && growsUpward) {",
  "if (isChatMode) {"
);

if (content.includes("if (isChatMode) {")) {
  console.log("Replaced isChatMode && growsUpward successfully.");
}

fs.writeFileSync('src/App.tsx', content);
