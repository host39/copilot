import { SchedulePreview } from "@web/components/Schedule/RuleFields";
import type { Proposal, ScheduleContext } from "@web/routes/schedule/rules";

/**
 * The details of a `schedule_prompt` approval card: the rule in plain words with the next 3 runs
 * in the user's time, the prompt, the model, where it runs, and the tools it would use without
 * asking.
 */
export function ScheduleProposal({
  proposal,
  context,
}: {
  proposal: Proposal;
  context: ScheduleContext | undefined;
}) {
  return (
    <div className="space-y-2">
      <p className="text-muted">{`The assistant wants to create the schedule "${proposal.name}".`}</p>
      <SchedulePreview cron={proposal.cron} timezone={proposal.timezone} />
      <dl className="grid gap-x-3 gap-y-1 text-sm sm:grid-cols-[auto_1fr]">
        <dt className="text-muted">Prompt</dt>
        <dd className="whitespace-pre-wrap break-words">{proposal.prompt}</dd>
        <dt className="text-muted">Model</dt>
        <dd>{context?.model ?? "This chat's model"}</dd>
        <dt className="text-muted">Runs in</dt>
        <dd>{context?.target ?? "This chat's project"}</dd>
        <dt className="text-muted">Tools without asking</dt>
        <dd>{context ? context.autoTools || "None" : "This project's tools set to Auto"}</dd>
      </dl>
    </div>
  );
}
