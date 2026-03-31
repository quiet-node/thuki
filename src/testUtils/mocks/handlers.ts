import { http, HttpResponse } from 'msw';

const OLLAMA_URL = 'http://127.0.0.1:11434';

function ollamaStreamResponse(tokens: string[]) {
  const lines = tokens
    .map((t) => JSON.stringify({ response: t, done: false }))
    .concat(JSON.stringify({ response: '', done: true }));
  return new HttpResponse(lines.join('\n'), {
    headers: { 'Content-Type': 'application/x-ndjson' },
  });
}

export const handlers = [
  http.post(`${OLLAMA_URL}/api/generate`, () => {
    return ollamaStreamResponse(['Hello', ' world', '!']);
  }),
];
