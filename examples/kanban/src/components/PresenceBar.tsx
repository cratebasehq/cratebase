import type { PresenceRecord } from "../types";
import { Avatar } from "./Avatar";

export function PresenceBar({ peers, selfUserId }: { peers: Map<string, PresenceRecord>; selfUserId: string }) {
  const list = [...peers.values()].sort((a, b) => a.userId.localeCompare(b.userId));

  return (
    <div className="presence-bar" title={`${list.length} online`}>
      <div className="presence-stack">
        {list.map((peer) => (
          <span className="presence-avatar" key={peer.userId} title={peer.userId === selfUserId ? `${peer.name} (you)` : peer.name}>
            <Avatar id={peer.userId} />
          </span>
        ))}
      </div>
      <span className="presence-count">
        <span className="presence-dot" />
        {list.length} online
      </span>
    </div>
  );
}
