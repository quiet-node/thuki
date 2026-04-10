const fs = require('fs');
let code = fs.readFileSync('src/__tests__/App.test.tsx', 'utf8');

const oldCode = `    const outer = document.querySelector('.justify-start');
    expect(outer).not.toBeNull();
    expect(document.querySelector('.justify-end')).toBeNull();`;

const newCode = `    const outer = document.querySelector('.justify-end');
    expect(outer).not.toBeNull();
    expect(document.querySelector('.justify-start')).toBeNull();`;

code = code.replace(oldCode, newCode);
fs.writeFileSync('src/__tests__/App.test.tsx', code);
