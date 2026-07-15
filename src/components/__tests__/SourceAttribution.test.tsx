import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import {
  SourceAttribution,
  renderAttributionMarkdown,
} from '../SourceAttribution';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}));

describe('renderAttributionMarkdown', () => {
  it('returns plain text when there are no links', () => {
    const nodes = renderAttributionMarkdown('no links here');
    expect(nodes).toHaveLength(1);
  });

  it('splits text around a markdown link', () => {
    const nodes = renderAttributionMarkdown(
      'Hello [Open-Meteo](https://open-meteo.com/) world',
    );
    expect(nodes.length).toBeGreaterThan(1);
  });

  it('handles attribution that ends exactly on a link', () => {
    const nodes = renderAttributionMarkdown(
      '[Weather data by Open-Meteo.com](https://open-meteo.com/)',
    );
    expect(nodes.length).toBe(1);
  });
});

describe('SourceAttribution', () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it('renders linked label and opens via open_url', () => {
    render(
      <SourceAttribution markdown="[Weather data by Open-Meteo.com](https://open-meteo.com/) (CC BY 4.0)" />,
    );
    const btn = screen.getByRole('button', {
      name: 'Weather data by Open-Meteo.com',
    });
    fireEvent.click(btn);
    expect(invoke).toHaveBeenCalledWith('open_url', {
      url: 'https://open-meteo.com/',
    });
    expect(screen.getByTestId('source-attribution').textContent).toContain(
      'CC BY 4.0',
    );
  });

  it('renders Wikipedia CC BY-SA licence link', () => {
    render(
      <SourceAttribution markdown="Source: Wikipedia ([CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/))" />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'CC BY-SA 4.0' }));
    expect(invoke).toHaveBeenCalledWith('open_url', {
      url: 'https://creativecommons.org/licenses/by-sa/4.0/',
    });
  });
});
