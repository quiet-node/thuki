import { render, screen, fireEvent, act } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { HistoryPanel } from '../HistoryPanel';
import type { ConversationSummary } from '../../types/history';

const NOW = Date.now();
const YESTERDAY = NOW - 86_400_000;
const OLDER = NOW - 86_400_000 * 3;

const CONVERSATIONS: ConversationSummary[] = [
  {
    id: 'c1',
    title: 'React basics',
    model: 'gemma3:4b',
    updated_at: NOW,
    message_count: 4,
  },
  {
    id: 'c2',
    title: 'Python bug fix',
    model: 'gemma3:4b',
    updated_at: YESTERDAY,
    message_count: 6,
  },
  {
    id: 'c3',
    title: 'Old topic',
    model: 'gemma3:4b',
    updated_at: OLDER,
    message_count: 2,
  },
];

function makeProps(
  overrides: Partial<Parameters<typeof HistoryPanel>[0]> = {},
) {
  return {
    listConversations: vi.fn(async () => CONVERSATIONS),
    onLoadConversation: vi.fn(),
    onSaveAndLoad: vi.fn(),
    onDeleteConversation: vi.fn(),
    hasCurrentMessages: false,
    showNewConversation: false,
    onNewConversation: vi.fn(),
    ...overrides,
  };
}

describe('HistoryPanel', () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('renders a search input focused on mount', async () => {
    const props = makeProps();
    render(<HistoryPanel {...props} />);

    await act(async () => {});

    const input = screen.getByPlaceholderText(/search/i);
    expect(input).toBeInTheDocument();
  });

  it('fetches conversations on mount', async () => {
    const props = makeProps();
    render(<HistoryPanel {...props} />);

    await act(async () => {});

    expect(props.listConversations).toHaveBeenCalledWith(undefined);
    expect(screen.getByText('React basics')).toBeInTheDocument();
  });

  it('groups conversations by date: Today and Yesterday labels appear', async () => {
    const props = makeProps();
    render(<HistoryPanel {...props} />);

    await act(async () => {});

    expect(screen.getByText('Today')).toBeInTheDocument();
    expect(screen.getByText('Yesterday')).toBeInTheDocument();
  });

  it('shows "No conversations yet" when list is empty', async () => {
    const props = makeProps({ listConversations: vi.fn(async () => []) });
    render(<HistoryPanel {...props} />);

    await act(async () => {});

    expect(screen.getByText(/no conversations yet/i)).toBeInTheDocument();
  });

  it('filters conversations by search with debounce', async () => {
    const listFn = vi.fn(async () => CONVERSATIONS);
    const props = makeProps({ listConversations: listFn });
    render(<HistoryPanel {...props} />);

    await act(async () => {});
    listFn.mockClear();

    const input = screen.getByPlaceholderText(/search/i);
    fireEvent.change(input, { target: { value: 'react' } });

    // debounce not yet fired
    expect(listFn).not.toHaveBeenCalled();

    await act(async () => {
      vi.advanceTimersByTime(250);
    });

    expect(listFn).toHaveBeenCalledWith('react');
  });

  it('passes undefined (not empty string) to listConversations when search is cleared', async () => {
    const listFn = vi.fn(async () => CONVERSATIONS);
    const props = makeProps({ listConversations: listFn });
    render(<HistoryPanel {...props} />);

    await act(async () => {});
    listFn.mockClear();

    const input = screen.getByPlaceholderText(/search/i);

    // Type something, then clear it
    fireEvent.change(input, { target: { value: 'react' } });
    fireEvent.change(input, { target: { value: '' } });

    await act(async () => {
      vi.advanceTimersByTime(250);
    });

    // Empty string maps to `undefined` so listFn receives no search arg
    expect(listFn).toHaveBeenLastCalledWith(undefined);
  });

  it('calls onLoadConversation when no current messages', async () => {
    const props = makeProps({ hasCurrentMessages: false });
    render(<HistoryPanel {...props} />);

    await act(async () => {});

    fireEvent.click(screen.getByRole('button', { name: /react basics/i }));
    expect(props.onLoadConversation).toHaveBeenCalledWith('c1');
  });

  it('shows SwitchConfirmation and hides search when hasCurrentMessages is true and item clicked', async () => {
    const props = makeProps({ hasCurrentMessages: true });
    render(<HistoryPanel {...props} />);

    await act(async () => {});

    fireEvent.click(screen.getByRole('button', { name: /react basics/i }));

    expect(screen.getByText(/switch conversations/i)).toBeInTheDocument();
    expect(screen.queryByPlaceholderText(/search/i)).toBeNull();
    expect(props.onLoadConversation).not.toHaveBeenCalled();
  });

  it('calls onSaveAndLoad from SwitchConfirmation Save & Switch', async () => {
    const props = makeProps({ hasCurrentMessages: true });
    render(<HistoryPanel {...props} />);

    await act(async () => {});

    fireEvent.click(screen.getByRole('button', { name: /react basics/i }));
    fireEvent.click(screen.getByRole('button', { name: /save & switch/i }));

    expect(props.onSaveAndLoad).toHaveBeenCalledWith('c1');
  });

  it('calls onLoadConversation from SwitchConfirmation Just Switch', async () => {
    const props = makeProps({ hasCurrentMessages: true });
    render(<HistoryPanel {...props} />);

    await act(async () => {});

    fireEvent.click(screen.getByRole('button', { name: /react basics/i }));
    fireEvent.click(screen.getByRole('button', { name: /just switch/i }));

    expect(props.onLoadConversation).toHaveBeenCalledWith('c1');
  });

  it('dismisses SwitchConfirmation when Cancel is clicked', async () => {
    const props = makeProps({ hasCurrentMessages: true });
    render(<HistoryPanel {...props} />);

    await act(async () => {});

    fireEvent.click(screen.getByRole('button', { name: /react basics/i }));
    expect(screen.getByText(/switch conversations/i)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /cancel/i }));
    expect(screen.queryByText(/switch conversations/i)).toBeNull();
  });

  it('calls onDeleteConversation when delete is clicked', async () => {
    const props = makeProps();
    render(<HistoryPanel {...props} />);

    await act(async () => {});

    const deleteButtons = screen.getAllByRole('button', { name: /delete/i });
    fireEvent.click(deleteButtons[0]);

    expect(props.onDeleteConversation).toHaveBeenCalledWith('c1');
  });

  it('removes the deleted conversation from the list optimistically', async () => {
    const props = makeProps();
    render(<HistoryPanel {...props} />);

    await act(async () => {});
    expect(screen.getByText('React basics')).toBeInTheDocument();

    const deleteButtons = screen.getAllByRole('button', { name: /delete/i });
    await act(async () => {
      fireEvent.click(deleteButtons[0]);
    });

    expect(screen.queryByText('React basics')).toBeNull();
  });

  it('hides New Conversation footer when showNewConversation is false', async () => {
    const props = makeProps({ showNewConversation: false });
    render(<HistoryPanel {...props} />);

    await act(async () => {});

    expect(
      screen.queryByRole('button', { name: /new conversation/i }),
    ).toBeNull();
  });

  it('shows New Conversation footer when showNewConversation is true', async () => {
    const props = makeProps({ showNewConversation: true });
    render(<HistoryPanel {...props} />);

    await act(async () => {});

    expect(
      screen.getByRole('button', { name: /new conversation/i }),
    ).toBeInTheDocument();
  });

  it('calls onNewConversation when footer button is clicked', async () => {
    const props = makeProps({ showNewConversation: true });
    render(<HistoryPanel {...props} />);

    await act(async () => {});

    fireEvent.click(screen.getByRole('button', { name: /new conversation/i }));
    expect(props.onNewConversation).toHaveBeenCalledOnce();
  });

  it('debounce: rapid successive keystrokes cancel previous timer and fire once', async () => {
    const listFn = vi.fn(async () => CONVERSATIONS);
    const props = makeProps({ listConversations: listFn });
    render(<HistoryPanel {...props} />);

    await act(async () => {});
    listFn.mockClear();

    const input = screen.getByPlaceholderText(/search/i);

    // First keystroke starts a debounce timer
    fireEvent.change(input, { target: { value: 'r' } });
    // Second keystroke before debounce fires — clears the first timer (line 130)
    fireEvent.change(input, { target: { value: 're' } });

    // Only after debounce delay should listFn be called — once, with 're'
    await act(async () => {
      vi.advanceTimersByTime(250);
    });

    expect(listFn).toHaveBeenCalledTimes(1);
    expect(listFn).toHaveBeenCalledWith('re');
  });

  it('does not call onLoadConversation when clicking the current conversation', async () => {
    const props = makeProps({
      hasCurrentMessages: false,
      currentConversationId: 'c1',
    });
    render(<HistoryPanel {...props} />);

    await act(async () => {});

    fireEvent.click(screen.getByRole('button', { name: /react basics/i }));
    expect(props.onLoadConversation).not.toHaveBeenCalled();
  });

  it('restores deleted conversation when onDeleteConversation rejects', async () => {
    // Bug: optimistic removal has no rollback — if the backend delete fails the
    // item disappears from the UI but still exists in SQLite, reappearing on next open.
    const props = makeProps({
      onDeleteConversation: vi.fn(async () => {
        throw new Error('delete failed');
      }),
    });
    render(<HistoryPanel {...props} />);

    await act(async () => {});

    expect(screen.getByText('React basics')).toBeInTheDocument();

    const deleteButtons = screen.getAllByRole('button', { name: /delete/i });
    await act(async () => {
      fireEvent.click(deleteButtons[0]);
    });

    // After the backend rejects, the conversation must be restored to the list
    expect(screen.getByText('React basics')).toBeInTheDocument();
  });

  it('shows SwitchConfirmation and hides search when pendingNewConversation is true', async () => {
    const onSaveAndNew = vi.fn();
    const onJustNew = vi.fn();
    const onCancelNew = vi.fn();
    const props = makeProps({
      pendingNewConversation: true,
      onSaveAndNew,
      onJustNew,
      onCancelNew,
      hasCurrentMessages: true,
    });
    render(<HistoryPanel {...props} />);

    await act(async () => {});

    // Search box should NOT be rendered
    expect(screen.queryByPlaceholderText(/search/i)).toBeNull();

    // SwitchConfirmation should show "new" variant text
    expect(screen.getByText('New conversation?')).toBeInTheDocument();

    // Save & Start New calls onSaveAndNew
    fireEvent.click(screen.getByRole('button', { name: /save & start new/i }));
    expect(onSaveAndNew).toHaveBeenCalledOnce();

    // Start New calls onJustNew
    fireEvent.click(screen.getByRole('button', { name: /^start new$/i }));
    expect(onJustNew).toHaveBeenCalledOnce();

    // Cancel calls onCancelNew
    fireEvent.click(screen.getByRole('button', { name: /cancel/i }));
    expect(onCancelNew).toHaveBeenCalledOnce();
  });

  it('shows error message when listConversations rejects', async () => {
    const props = makeProps({
      listConversations: vi.fn(async () => {
        throw new Error('DB error');
      }),
    });
    render(<HistoryPanel {...props} />);

    await act(async () => {});

    expect(screen.getByText(/couldn't load history/i)).toBeInTheDocument();
  });
});
