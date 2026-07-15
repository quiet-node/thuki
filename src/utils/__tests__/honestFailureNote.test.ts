import { describe, it, expect } from 'vitest';
import {
  HONEST_FAILURE_NOTE_BODY,
  SEARCH_NO_RESULTS_NOTE_BODY,
  SEARCH_UNREACHABLE_NOTE_BODY,
  searchFailNoteBody,
  splitHonestFailureNote,
} from '../honestFailureNote';

describe('splitHonestFailureNote', () => {
  it('splits body + blank-line + note', () => {
    const content = `Fake number 999.\n\n${HONEST_FAILURE_NOTE_BODY}`;
    expect(splitHonestFailureNote(content)).toEqual({
      body: 'Fake number 999.',
      note: HONEST_FAILURE_NOTE_BODY,
    });
  });

  it('returns body only when note absent', () => {
    const content = 'Normal answer with [1] citation.';
    expect(splitHonestFailureNote(content)).toEqual({
      body: content,
      note: null,
    });
  });

  it('returns note-only when answer is just the honesty note', () => {
    expect(splitHonestFailureNote(HONEST_FAILURE_NOTE_BODY)).toEqual({
      body: '',
      note: HONEST_FAILURE_NOTE_BODY,
    });
  });

  it('accepts legacy markdown italic wrappers', () => {
    const wrapped = `*${HONEST_FAILURE_NOTE_BODY}*`;
    const content = `Claim text.\n\n${wrapped}`;
    expect(splitHonestFailureNote(content)).toEqual({
      body: 'Claim text.',
      note: HONEST_FAILURE_NOTE_BODY,
    });
  });

  it('returns note-only for legacy wrapped-only content', () => {
    expect(splitHonestFailureNote(`*${HONEST_FAILURE_NOTE_BODY}*`)).toEqual({
      body: '',
      note: HONEST_FAILURE_NOTE_BODY,
    });
  });

  it('ignores note body mid-answer (not trailing)', () => {
    const content = `${HONEST_FAILURE_NOTE_BODY}\n\nMore after.`;
    expect(splitHonestFailureNote(content)).toEqual({
      body: content,
      note: null,
    });
  });

  it('preserves empty string', () => {
    expect(splitHonestFailureNote('')).toEqual({ body: '', note: null });
  });

  it('trims trailing whitespace on match without mutating body interior', () => {
    const content = `Line one.\n\n${HONEST_FAILURE_NOTE_BODY}  \n`;
    expect(splitHonestFailureNote(content)).toEqual({
      body: 'Line one.',
      note: HONEST_FAILURE_NOTE_BODY,
    });
  });

  const LEGACY_NOTE_BODY =
    "Thuki found sources but could not verify the answer's citations against the page text. Treat specific claims carefully, or try rephrasing or a larger model in Settings.";

  it('accepts the pre-shortening legacy body, restyled with the current note', () => {
    const content = `Fake number 999.\n\n${LEGACY_NOTE_BODY}`;
    expect(splitHonestFailureNote(content)).toEqual({
      body: 'Fake number 999.',
      note: HONEST_FAILURE_NOTE_BODY,
    });
  });

  it('returns note-only for pre-shortening legacy-only content', () => {
    expect(splitHonestFailureNote(LEGACY_NOTE_BODY)).toEqual({
      body: '',
      note: HONEST_FAILURE_NOTE_BODY,
    });
  });

  it('accepts the pre-shortening legacy body wrapped in markdown italics', () => {
    const wrapped = `*${LEGACY_NOTE_BODY}*`;
    const content = `Claim text.\n\n${wrapped}`;
    expect(splitHonestFailureNote(content)).toEqual({
      body: 'Claim text.',
      note: HONEST_FAILURE_NOTE_BODY,
    });
  });
});

describe('searchFailNoteBody', () => {
  it('maps unreachable to the connection-check copy', () => {
    expect(searchFailNoteBody('unreachable')).toBe(
      SEARCH_UNREACHABLE_NOTE_BODY,
    );
    expect(searchFailNoteBody('unreachable')).toContain(
      'Check your internet connection',
    );
  });

  it('maps no_results to the rephrase copy', () => {
    expect(searchFailNoteBody('no_results')).toBe(SEARCH_NO_RESULTS_NOTE_BODY);
    expect(searchFailNoteBody('no_results')).toContain('Try rephrasing');
  });
});
