/**
 * Walk up from `fromEl` and hard-pin the chat messages scroller to its bottom.
 *
 * Finds the nearest ancestor with class `chat-messages-scroll` and sets
 * `scrollTop = scrollHeight`. Used when search sources (or other chrome)
 * grow under a long answer so `scrollIntoView` heuristics are never trusted.
 *
 * No-ops when `fromEl` is null or no matching ancestor exists.
 */
export function pinChatMessagesToBottom(fromEl: HTMLElement | null): void {
  if (!fromEl) return;

  let node: HTMLElement | null = fromEl;
  while (node) {
    if (node.classList.contains('chat-messages-scroll')) {
      node.scrollTop = node.scrollHeight;
      return;
    }
    node = node.parentElement;
  }
}
