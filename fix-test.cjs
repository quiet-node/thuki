const fs = require('fs');
let code = fs.readFileSync('src/__tests__/App.test.tsx', 'utf8');

const oldTest = `  it('applies justify-start when window has room below', async () => {
    vi.mocked(getCurrentWindow().outerPosition).mockResolvedValue(
      new PhysicalPosition(100, 100), // far from bottom
    );

    render(<App />);

    await act(async () => {
      // Trigger visibility event
      eventCbs['thuki://visibility']({
        payload: { state: 'show', selected_text: null, window_anchor: null },
      });
    });

    const outer = document.querySelector('.justify-start');
    expect(outer).not.toBeNull();
    expect(document.querySelector('.justify-end')).toBeNull();
  });`;

const newTest = `  it('applies justify-end to enforce upward growth morphing', async () => {
    vi.mocked(getCurrentWindow().outerPosition).mockResolvedValue(
      new PhysicalPosition(100, 100), // far from bottom
    );

    render(<App />);

    await act(async () => {
      // Trigger visibility event
      eventCbs['thuki://visibility']({
        payload: { state: 'show', selected_text: null, window_anchor: null },
      });
    });

    const outer = document.querySelector('.justify-end');
    expect(outer).not.toBeNull();
    expect(document.querySelector('.justify-start')).toBeNull();
  });`;

code = code.replace(oldTest, newTest);
fs.writeFileSync('src/__tests__/App.test.tsx', code);
