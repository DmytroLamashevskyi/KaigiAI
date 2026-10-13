import { useApp } from "../../state/AppState";
import { formatMs, latencyStats } from "../../state/latency";
import { Field, Section } from "./common";

/** Diagnostics (§10.16): toggles the latency breakdown on message-time hover and
 *  shows median / p95 per pipeline stage for the active conversation. All
 *  numbers are local — nothing is sent anywhere. */
export default function DiagnosticsSection() {
  const { settings, updateSettings, activeConversation, activeMessages } = useApp();
  const { count, stages } = latencyStats(activeMessages);

  return (
    <Section title="Диагностика">
      <Field
        label="Режим диагностики"
        hint="При наведении на время реплики показывает, сколько заняла каждая стадия: пауза, распознавание, перевод. Данные хранятся только локально."
      >
        <button
          className={"switch" + (settings.diagnostics ? " on" : "")}
          onClick={() => updateSettings({ diagnostics: !settings.diagnostics })}
        >
          <span className="switch-knob" />
        </button>
      </Field>
      {settings.diagnostics && (
        <div className="latency-summary">
          <div className="field-hint">
            {activeConversation
              ? `Задержка в беседе «${activeConversation.title}» — реплик с замерами: ${count}`
              : "Откройте беседу, чтобы увидеть сводку задержки."}
          </div>
          {count > 0 && (
            <table className="latency-table">
              <thead>
                <tr>
                  <th>Стадия</th>
                  <th>Медиана</th>
                  <th>p95</th>
                </tr>
              </thead>
              <tbody>
                {stages.map((s) => (
                  <tr key={s.label}>
                    <td>{s.label}</td>
                    <td>{formatMs(s.median)}</td>
                    <td>{formatMs(s.p95)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      )}
    </Section>
  );
}
