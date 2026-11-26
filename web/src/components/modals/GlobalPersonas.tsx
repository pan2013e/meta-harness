import { useState } from "react";
import { Pencil, Plus, Trash2 } from "lucide-react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import type { GlobalPersona } from "../../types/api";
import { PersonaModelFields } from "./PersonaModelFields";

const fieldClass = "w-full min-w-0 rounded-lg border border-line bg-inset px-2.5 py-1.5 text-ink";
const actionClass = "rounded-lg border border-line bg-elevated px-2.5 py-1.5 text-ink-soft hover:bg-hover disabled:opacity-50";

export function GlobalPersonas() {
  const {
    adapters, adapterCatalogs, probes, globalPersonas, saveGlobalPersona, deleteGlobalPersona,
    activeSession, members, selectSession,
  } = useAppStore();
  const [editor, setEditor] = useState<{ persona: GlobalPersona; creating: boolean } | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const adapterIds = Object.keys(adapters);

  const run = async (action: () => Promise<void>) => {
    setSaving(true);
    setError(null);
    try {
      await action();
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setSaving(false);
    }
  };

  const addToSession = (persona: GlobalPersona) => {
    const id = activeSession?.id;
    if (!id) return;
    void run(async () => {
      await api.addGlobalPersonaToSession(id, persona.handle);
      if (useAppStore.getState().activeSession?.id === id) await selectSession(id);
    });
  };

  const startCreating = () => setEditor({
    creating: true,
    persona: {
      handle: "",
      adapter_id: probes.find((probe) => probe.installed)?.adapter_id ?? adapterIds[0],
      display_name: "",
      persona_prompt: "",
      active_model: null,
      active_effort: null,
    },
  });

  const updateEditor = (values: Partial<GlobalPersona>) => {
    setEditor((current) => current && { ...current, persona: { ...current.persona, ...values } });
  };

  const save = async () => {
    if (!editor) return;
    const handle = editor.persona.handle.trim().replace(/^@+/, "").toLowerCase();
    if (editor.creating && globalPersonas.some((persona) => persona.handle === handle)) {
      setError("That persona already exists. Edit it from the list.");
      return;
    }
    await run(async () => {
      await saveGlobalPersona({ ...editor.persona, handle });
      setEditor(null);
    });
  };

  return (
    <section className="space-y-4 text-xs">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div>
          <h2 className="font-semibold text-ink">Global personas</h2>
          <p className="mt-1 text-ink-muted">New sessions include personas with available harnesses. You can also add them to the current session.</p>
        </div>
        <button type="button" className={actionClass} disabled={saving || !adapterIds.length}
          onClick={startCreating}>
          <Plus className="mr-1 inline h-3.5 w-3.5" /> New global persona
        </button>
      </div>
      {error && <p role="alert" className="text-neg">{error}</p>}
      {editor && (
        <form className="space-y-3 rounded-xl border border-line bg-inset p-3"
          onSubmit={(event) => { event.preventDefault(); void save(); }}>
          <div className="grid gap-2 sm:grid-cols-3">
            <label className="space-y-1">
              <span>Handle</span>
              <input className={fieldClass} placeholder="@reviewer" required maxLength={49}
                value={editor.persona.handle} disabled={!editor.creating || saving}
                onChange={(event) => updateEditor({ handle: event.target.value })} />
            </label>
            <label className="space-y-1">
              <span>Harness</span>
              <select className={fieldClass} value={editor.persona.adapter_id} disabled={saving}
                onChange={(event) => updateEditor({
                  adapter_id: event.target.value, active_model: null, active_effort: null,
                })}>
                {!adapters[editor.persona.adapter_id] && <option value={editor.persona.adapter_id}>{editor.persona.adapter_id}</option>}
                {adapterIds.map((id) => <option key={id} value={id}>{adapters[id].display_name}</option>)}
              </select>
            </label>
            <label className="space-y-1">
              <span>Display name</span>
              <input className={fieldClass} value={editor.persona.display_name} maxLength={128}
                disabled={saving} onChange={(event) => updateEditor({ display_name: event.target.value })} />
            </label>
          </div>
          <PersonaModelFields label="Global persona" catalog={adapterCatalogs[editor.persona.adapter_id]}
            model={editor.persona.active_model} effort={editor.persona.active_effort}
            onChange={updateEditor} disabled={saving} />
          <label className="block space-y-1">
            <span>Instructions</span>
            <textarea className={fieldClass} rows={3} value={editor.persona.persona_prompt} maxLength={16000}
              disabled={saving} onChange={(event) => updateEditor({ persona_prompt: event.target.value })} />
          </label>
          <div className="flex justify-end gap-2">
            <button type="button" className={actionClass} onClick={() => setEditor(null)} disabled={saving}>Cancel</button>
            <button type="submit" className="rounded-lg bg-brand px-3 py-1.5 font-medium text-white disabled:opacity-50"
              disabled={saving || !editor.persona.handle.trim()}>Save persona</button>
          </div>
        </form>
      )}
      {globalPersonas.map((persona) => {
        const installed = probes.some((probe) => probe.adapter_id === persona.adapter_id && probe.installed);
        const inSession = members.some((member) => member.handle.toLowerCase() === persona.handle);
        return (
          <article key={persona.handle} className="space-y-2 rounded-xl border border-line bg-inset px-3 py-2.5">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="min-w-0">
                <h3 className="break-words font-medium text-ink">{persona.display_name}</h3>
                <p className="break-words text-ink-muted">@{persona.handle} · {adapters[persona.adapter_id]?.display_name ?? persona.adapter_id}</p>
              </div>
              <div className="flex flex-wrap items-center gap-1">
                {activeSession && !activeSession.archived_at && (
                  <button type="button" className={actionClass} disabled={saving || inSession || !installed}
                    onClick={() => addToSession(persona)}>
                    {inSession ? "In session" : installed ? "Add to session" : "Harness unavailable"}
                  </button>
                )}
                <button type="button" aria-label={"Edit global persona @" + persona.handle} disabled={saving}
                  className="rounded-lg p-1.5 text-ink-muted hover:bg-hover hover:text-ink"
                  onClick={() => setEditor({ persona, creating: false })}>
                  <Pencil className="h-3.5 w-3.5" />
                </button>
                <button type="button" aria-label={"Delete global persona @" + persona.handle} disabled={saving}
                  className="rounded-lg p-1.5 text-ink-muted hover:bg-neg-soft hover:text-neg"
                  onClick={() => void run(async () => {
                    await deleteGlobalPersona(persona.handle);
                    if (editor?.persona.handle === persona.handle) setEditor(null);
                  })}>
                  <Trash2 className="h-3.5 w-3.5" />
                </button>
              </div>
            </div>
            {(persona.active_model || persona.active_effort) && (
              <p className="break-words text-ink-soft">{[persona.active_model, persona.active_effort].filter(Boolean).join(" · ")}</p>
            )}
            {persona.persona_prompt && <p className="line-clamp-2 break-words text-ink-muted">{persona.persona_prompt}</p>}
          </article>
        );
      })}
      {!globalPersonas.length && !editor && <p className="rounded-xl border border-line p-3 text-ink-muted">No global personas yet.</p>}
    </section>
  );
}
