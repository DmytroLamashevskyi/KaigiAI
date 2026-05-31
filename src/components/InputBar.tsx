import { useState } from "react";
import { useApp } from "../state/AppState";
import { useT } from "../i18n/useT";

export default function InputBar() {
  const { addTextMessage } = useApp();
  const t = useT();
  const [text, setText] = useState("");

  const send = () => {
    if (!text.trim()) return;
    addTextMessage(text);
    setText("");
  };

  return (
    <div className="input-bar">
      <textarea
        className="input-field"
        placeholder={t("input.placeholder")}
        value={text}
        rows={1}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            send();
          }
        }}
      />
      <button className="send-btn" onClick={send} disabled={!text.trim()}>
        ↑
      </button>
    </div>
  );
}
