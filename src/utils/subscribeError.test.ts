import { describe, it, expect } from 'vitest';
import { subscribeErrorMessage } from './subscribeError';

describe('subscribeErrorMessage', () => {
  it('gives a rate-limit-specific line for the 429 discriminant', () => {
    expect(subscribeErrorMessage('rate_limited')).toMatch(/too many requests/i);
  });

  it('falls back to a generic retryable line for any other rejection', () => {
    expect(
      subscribeErrorMessage('Something went wrong. Please try again later.'),
    ).toMatch(/couldn't send right now/i);
    expect(subscribeErrorMessage(new Error('network'))).toMatch(
      /couldn't send right now/i,
    );
    expect(subscribeErrorMessage(undefined)).toMatch(
      /couldn't send right now/i,
    );
  });
});
