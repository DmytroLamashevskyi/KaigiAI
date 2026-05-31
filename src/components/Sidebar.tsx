import { useState } from "react";
import { useApp } from "../state/AppState";
import { languageName } from "../data/languages";
import { useT } from "../i18n/useT";

export default function Sidebar() {
  const {
    conversations,
    activeId,
    view,
    selectConversation,
    newConversation,
    openSettings,
    renameConversation,
    deleteConversation,
    downloadConversation,
  } = useApp();
  const t = useT();

  const [menuId, setMenuId] = useState<string | null>(null);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [draft, setDraft] = useState("");

  const startRename = (id: string, title: string) => {
    setMenuId(null);
    setDraft(title);
    setRenamingId(id);
  };
  const commitRename = () => {
    if (renamingId) renameConversation(renamingId, draft);
    setRenamingId(null);
  };

  return (
    <aside className="sidebar">
      {menuId && (
        <div className="menu-backdrop" onClick={() => setMenuId(null)} />
      )}

      <div className="sidebar-header">
        <span className="brand">KaigiAI</span>
      </div>

      <button className="new-btn" onClick={newConversation}>
        <span className="plus">+</span> {t("nav.newDialog")}
      </button>

      <div className="conv-list">
        {conversations.map((c) => (
          <div key={c.id} className="conv-item-wrap">
            {renamingId === c.id ? (
              <input
                className="conv-rename"
                value={draft}
                autoFocus
                onChange={(e) => setDraft(e.target.value)}
                onBlur={commitRename}
                onKeyDown={(e) => {
                  if (e.key === "Enter") commitRename();
                  if (e.key === "Escape") setRenamingId(null);
                }}
              />
            ) : (
              <button
                className={
                  "conv-item" +
                  (c.id === activeId && view === "transcript" ? " active" : "")
                }
                onClick={() => selectConversation(c.id)}
              >
                <span className="conv-title">{c.title}</span>
                <span className="conv-langs">
                  {languageName(c.langA)} ↔ {languageName(c.langB)}
                </span>
              </button>
            )}

            <button
              className="conv-menu-btn"
              title={t("menu.actions")}
              onClick={() => setMenuId((m) => (m === c.id ? null : c.id))}
            >
              ⋯
            </button>

            {menuId === c.id && (
              <div className="conv-menu">
                <button onClick={() => startRename(c.id, c.title)}>
                  {t("menu.rename")}
                </button>
                <button
                  onClick={() => {
                    downloadConversation(c.id);
                    setMenuId(null);
                  }}
                >
                  {t("menu.download")}
                </button>
                <button
                  className="danger"
                  onClick={() => {
                    deleteConversation(c.id);
                    setMenuId(null);
                  }}
                >
                  {t("menu.delete")}
                </button>
              </div>
            )}
          </div>
        ))}
        {conversations.length === 0 && (
          <p className="empty-hint">{t("nav.noDialogs")}</p>
        )}
      </div>

      <button
        className={"settings-btn" + (view === "settings" ? " active" : "")}
        onClick={openSettings}
      >
        <span className="gear">⚙</span> {t("nav.settings")}
      </button>
    </aside>
  );
}
