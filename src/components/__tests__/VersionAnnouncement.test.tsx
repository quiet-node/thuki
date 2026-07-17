import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { VersionAnnouncement } from '../VersionAnnouncement';
import {
  V016_AUTO_SEARCH_ANNOUNCEMENT,
  AUTO_SEARCH_PUBLIC_BLOG_POST_URL,
  v016AutoSearchSettingsCta,
} from '../../config/versionAnnouncements';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}));

describe('VersionAnnouncement', () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it('renders title, body, learn link, and actions', () => {
    const onPrimary = vi.fn();
    const onSecondary = vi.fn();
    render(
      <VersionAnnouncement
        title={V016_AUTO_SEARCH_ANNOUNCEMENT.title}
        body={V016_AUTO_SEARCH_ANNOUNCEMENT.body}
        learn={{
          ...V016_AUTO_SEARCH_ANNOUNCEMENT.learn,
          testId: 'version-announcement-learn',
        }}
        actions={[
          {
            label: 'Acknowledge',
            onClick: onPrimary,
            variant: 'primary',
            testId: 'version-announcement-primary',
          },
          {
            label: v016AutoSearchSettingsCta(true),
            onClick: onSecondary,
            variant: 'secondary',
            testId: 'version-announcement-secondary',
          },
        ]}
      />,
    );
    expect(screen.getByText(V016_AUTO_SEARCH_ANNOUNCEMENT.title)).toBeTruthy();
    expect(
      screen.getByText(V016_AUTO_SEARCH_ANNOUNCEMENT.body, { exact: false }),
    ).toBeTruthy();
    expect(
      screen.getByText(V016_AUTO_SEARCH_ANNOUNCEMENT.learn.label),
    ).toBeTruthy();
    fireEvent.click(screen.getByTestId('version-announcement-primary'));
    expect(onPrimary).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByTestId('version-announcement-secondary'));
    expect(onSecondary).toHaveBeenCalledTimes(1);
  });

  it('opens learn URL via open_url', () => {
    render(
      <VersionAnnouncement
        title="T"
        body="B"
        learn={{
          label: 'Learn ↗',
          url: AUTO_SEARCH_PUBLIC_BLOG_POST_URL,
          testId: 'learn',
        }}
        actions={[]}
      />,
    );
    fireEvent.click(screen.getByTestId('learn'));
    expect(invoke).toHaveBeenCalledWith('open_url', {
      url: AUTO_SEARCH_PUBLIC_BLOG_POST_URL,
    });
  });

  it('v016AutoSearchSettingsCta reflects auto_search state', () => {
    expect(v016AutoSearchSettingsCta(true)).toBe('Turn off in Settings');
    expect(v016AutoSearchSettingsCta(false)).toBe('Turn on in Settings');
  });

  it('renders without learn when omitted', () => {
    render(
      <VersionAnnouncement
        title="T"
        body="Body only"
        actions={[
          {
            label: 'Acknowledge',
            onClick: () => {},
            variant: 'primary',
            testId: 'p',
          },
        ]}
      />,
    );
    expect(screen.getByText('Body only')).toBeTruthy();
    expect(screen.queryByTestId('version-announcement-learn')).toBeNull();
  });

  it('defaults the learn control test id from the root testId', () => {
    render(
      <VersionAnnouncement
        title="T"
        body="B"
        testId="va"
        learn={{ label: 'Learn ↗', url: AUTO_SEARCH_PUBLIC_BLOG_POST_URL }}
        actions={[]}
      />,
    );
    expect(screen.getByTestId('va-learn')).toBeInTheDocument();
  });
});
