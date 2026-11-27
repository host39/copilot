// Spoken replies on the chat page: the `audio.delta` events of a turn this page started play as
// they arrive while Speak replies is on. Turning it off, Stop, and leaving the page silence them.

import { Player } from "@web/audio/player";
import { useEffect, useMemo, useRef } from "react";

/** What a turn does with spoken audio. */
export type Speaker = {
  /** A turn starts from a click: silence the last reply and get audio ready to start. */
  begin: () => void;
  /** Plays one `audio.delta` of that turn. */
  play: (data: string, format: string) => void;
  /** Silences the reply. */
  stop: () => void;
  /** Makes audio ready to start, from a click (turning Speak replies on). */
  unlock: () => void;
};

/** The chat page's speaker; it plays only while `on` (Speak replies, on a model with `audioOut`). */
export function useSpeaker(on: boolean): Speaker {
  const player = useRef<Player | null>(null);
  const enabled = useRef(on);
  enabled.current = on;

  useEffect(() => {
    if (!on) player.current?.stop();
  }, [on]);

  useEffect(
    () => () => {
      player.current?.close();
      player.current = null;
    },
    []
  );

  return useMemo(() => {
    const get = () => {
      player.current ??= new Player();
      return player.current;
    };
    return {
      begin: () => {
        player.current?.stop();
        // Browsers let audio start only from a user gesture: make the context during the click.
        if (enabled.current) get().unlock();
      },
      play: (data, format) => {
        if (enabled.current) get().play(data, format);
      },
      stop: () => player.current?.stop(),
      unlock: () => get().unlock(),
    };
  }, []);
}
