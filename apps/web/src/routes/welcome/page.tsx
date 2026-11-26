import type { Preset, ProviderKind } from "@copilot/contracts";
import { useUser } from "@web/auth/session";
import Button from "@web/components/Button/Button";
import { TextField } from "@web/components/Form/TextField";
import { PageTitle } from "@web/components/Layout/PageTitle";
import path from "@web/routes/path";
import { useState } from "react";
import { LuCheck } from "react-icons/lu";
import { Link } from "react-router-dom";

type Choice = {
  id: string;
  label: string;
  kind: ProviderKind;
  preset: Preset;
  /** What the user types: a key, an address, or both. */
  asks: "key" | "url" | "both";
  urlHint?: string;
};

/** The spec's presets: a key for hosted providers, an address for Ollama and custom servers. */
const CHOICES: Choice[] = [
  { id: "openai", label: "OpenAI", kind: "openai", preset: "openai", asks: "key" },
  { id: "anthropic", label: "Anthropic", kind: "anthropic", preset: "anthropic", asks: "key" },
  { id: "gemini", label: "Gemini", kind: "openai", preset: "gemini", asks: "key" },
  { id: "openrouter", label: "OpenRouter", kind: "openai", preset: "openrouter", asks: "key" },
  {
    id: "ollama",
    label: "Ollama",
    kind: "openai",
    preset: "ollama",
    asks: "url",
    urlHint: "http://localhost:11434/v1",
  },
  {
    id: "custom-openai",
    label: "OpenAI-compatible",
    kind: "openai",
    preset: "custom",
    asks: "both",
    urlHint: "https://llm.example.com/v1",
  },
  {
    id: "custom-anthropic",
    label: "Anthropic-compatible",
    kind: "anthropic",
    preset: "custom",
    asks: "both",
    urlHint: "https://gateway.example.com",
  },
];

const STEPS = ["Connect a provider", "Test", "Choose models"] as const;

function Steps({ current }: { current: number }) {
  return (
    <ol className="flex gap-2 text-sm">
      {STEPS.map((label, index) => {
        const done = index < current;
        const active = index === current;
        return (
          <li
            key={label}
            aria-current={active ? "step" : undefined}
            className={`flex flex-1 flex-col gap-2 ${active ? "text-fg" : "text-muted"}`}
          >
            <span
              className={`h-1 rounded-full ${done || active ? "bg-accent" : "bg-line"}`}
              aria-hidden
            />
            <span className="flex items-center gap-1">
              {done ? <LuCheck aria-hidden className="text-accent" /> : null}
              <span className={active ? "font-medium" : ""}>{label}</span>
            </span>
          </li>
        );
      })}
    </ol>
  );
}

/**
 * First-run setup: (1) pick a preset and paste a key, or enter an Ollama address; (2) test it;
 * (3) choose models. This is the shell of the three steps; the provider routes behind steps 2
 * and 3 come with the Providers settings.
 */
export default function WelcomePage() {
  const user = useUser();
  const [step, setStep] = useState(0);
  const [choice, setChoice] = useState<Choice>(CHOICES[0] as Choice);

  return (
    <main className="min-h-dvh bg-surface px-4 py-8 sm:py-12">
      <PageTitle page="Welcome" />
      <div className="mx-auto w-full max-w-xl">
        <div className="flex items-center justify-between gap-4">
          <div className="flex items-center gap-2">
            <img src="/icons/icon.svg" alt="" className="size-8" />
            <span className="font-semibold tracking-tight">Copilot</span>
          </div>
          <Link to={path.home} className="text-link text-sm">
            Skip for now
          </Link>
        </div>

        <h1 className="mt-8 text-2xl font-semibold tracking-tight">Welcome, {user.displayName}</h1>
        <p className="mt-2 text-sm text-muted">
          Copilot forwards your chats to a model provider. Connect one to start.
        </p>

        <div className="mt-6">
          <Steps current={step} />
        </div>

        <div className="card-panel mt-6">
          {step === 0 ? (
            <form
              noValidate
              className="space-y-6"
              onSubmit={(event) => {
                event.preventDefault();
                setStep(1);
              }}
            >
              <fieldset>
                <legend className="field-label">Provider</legend>
                <div className="mt-2 grid grid-cols-2 gap-2 sm:grid-cols-3">
                  {CHOICES.map((option) => (
                    <label key={option.id} className="choice-card min-h-12 items-center">
                      <input
                        type="radio"
                        name="preset"
                        value={option.id}
                        checked={choice.id === option.id}
                        onChange={() => setChoice(option)}
                        className="sr-only"
                      />
                      {option.label}
                    </label>
                  ))}
                </div>
              </fieldset>

              {choice.asks !== "key" ? (
                <TextField
                  key={`${choice.id}-url`}
                  label="Server address"
                  name="baseUrl"
                  type="url"
                  inputMode="url"
                  autoComplete="off"
                  autoCapitalize="none"
                  spellCheck={false}
                  placeholder={choice.urlHint}
                />
              ) : null}
              {choice.asks !== "url" ? (
                <TextField
                  key={`${choice.id}-key`}
                  label={choice.asks === "both" ? "API key (optional)" : "API key"}
                  name="apiKey"
                  type="password"
                  autoComplete="off"
                  spellCheck={false}
                  hint="Stored encrypted on your server. It is never shown again."
                />
              ) : null}

              <div className="flex justify-end">
                <Button type="submit">Continue</Button>
              </div>
            </form>
          ) : (
            <div className="space-y-4">
              <h2 className="text-base font-semibold">{STEPS[step]}</h2>
              <p className="text-sm text-muted">
                Testing {choice.label} and choosing its models are not available in this build yet.
                You can add the provider later in Settings.
              </p>
              <div className="flex flex-wrap justify-between gap-2">
                <Button variant="secondary" onClick={() => setStep(step - 1)}>
                  Back
                </Button>
                {step < STEPS.length - 1 ? (
                  <Button onClick={() => setStep(step + 1)}>Continue</Button>
                ) : (
                  <Link to={path.home} className="text-link inline-flex min-h-10 items-center">
                    Go to Copilot
                  </Link>
                )}
              </div>
            </div>
          )}
        </div>
      </div>
    </main>
  );
}
