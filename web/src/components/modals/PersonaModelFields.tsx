import type { AdapterModelCatalog } from "../../types/api";

export interface PersonaModel {
  active_model: string | null;
  active_effort: string | null;
}

export function PersonaModelFields({ label, catalog, model, effort, disabled, onChange }: {
  label: string;
  catalog?: AdapterModelCatalog;
  model: string | null;
  effort: string | null;
  disabled: boolean;
  onChange: (value: Partial<PersonaModel>) => void;
}) {
  const selectClass = "mt-1 w-full rounded-lg border border-line bg-inset px-2.5 py-1.5 text-ink disabled:opacity-50";
  const effortsFor = (value: string | null) =>
    catalog?.model_details?.find((choice) => choice.value === value)?.supported_efforts ?? catalog?.efforts ?? [];
  const models = catalog?.models ?? [];
  const efforts = effortsFor(model);
  return (
    <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
      <label className="text-xs text-ink-muted">
        Model
        <select aria-label={label + " model"} value={model ?? ""} disabled={disabled} className={selectClass}
          onChange={(event) => {
            const active_model = event.target.value || null;
            onChange({
              active_model,
              ...(effort && !effortsFor(active_model).some((choice) => choice.value === effort)
                ? { active_effort: null }
                : {}),
            });
          }}>
          <option value="">Inherit harness default</option>
          {model && !models.some((choice) => choice.value === model) && <option value={model}>{model}</option>}
          {models.map((choice) => <option key={choice.value} value={choice.value}>{choice.label}</option>)}
        </select>
      </label>
      <label className="text-xs text-ink-muted">
        Reasoning effort
        <select aria-label={label + " effort"} value={effort ?? ""} disabled={disabled} className={selectClass}
          onChange={(event) => onChange({ active_effort: event.target.value || null })}>
          <option value="">Inherit harness default</option>
          {effort && !efforts.some((choice) => choice.value === effort) && <option value={effort}>{effort}</option>}
          {efforts.map((choice) => <option key={choice.value} value={choice.value}>{choice.label}</option>)}
        </select>
      </label>
    </div>
  );
}
