import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { DownloadRiskConfirm } from './DownloadRiskConfirm';

describe('DownloadRiskConfirm', () => {
  it('renders the title and the at-your-own-risk body', () => {
    render(<DownloadRiskConfirm onConfirm={() => {}} onCancel={() => {}} />);
    expect(screen.getByText('Before you download')).toBeInTheDocument();
    expect(
      screen.getByText(
        'Models on Hugging Face can be low-quality, uncensored, or unsafe. Do your own research; download and run at your own risk.',
      ),
    ).toBeInTheDocument();
  });

  it('calls onConfirm when Download anyway is clicked', () => {
    const onConfirm = vi.fn();
    render(<DownloadRiskConfirm onConfirm={onConfirm} onCancel={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: 'Download anyway' }));
    expect(onConfirm).toHaveBeenCalledOnce();
  });

  it('calls onCancel when Cancel is clicked', () => {
    const onCancel = vi.fn();
    render(<DownloadRiskConfirm onConfirm={() => {}} onCancel={onCancel} />);
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(onCancel).toHaveBeenCalledOnce();
  });
});
