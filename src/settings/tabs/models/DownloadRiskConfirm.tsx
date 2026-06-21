/**
 * Inline pre-download confirmation for Browse-all (Hugging Face) models.
 *
 * Browse-all results are an unfiltered live fetch from Hugging Face, so the
 * download click is the point where the user accepts an unreviewed
 * third-party model. This card replaces the quant row's download control
 * until the user confirms or backs out. The copy and the at-your-own-risk
 * framing live here so the wording is owned in one place and unit-tested; the
 * card chrome reuses the download flow's primitives so the two read as one
 * family.
 */
import {
  ButtonRow,
  Card,
  Detail,
  FlowButton,
  Headline,
} from '../../../components/DownloadProgress';

export interface DownloadRiskConfirmProps {
  /** Proceed with the download. */
  onConfirm: () => void;
  /** Back out and restore the download control. */
  onCancel: () => void;
}

export function DownloadRiskConfirm({
  onConfirm,
  onCancel,
}: DownloadRiskConfirmProps) {
  return (
    <Card>
      <Headline>Before you download</Headline>
      <Detail>
        Models on Hugging Face can be low-quality, uncensored, or unsafe. Do
        your own research; download and run at your own risk.
      </Detail>
      <ButtonRow>
        <FlowButton label="Cancel" onClick={onCancel} />
        <FlowButton label="Download anyway" primary onClick={onConfirm} />
      </ButtonRow>
    </Card>
  );
}
