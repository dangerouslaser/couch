# Activities

An activity brings devices together for a task such as watching a movie. Configure it in **Activities → Create activity** in the web UI:

1. Choose a name, room, and included devices. Devices may come from other rooms.
2. Under **Devices & sequences**, build the **On sequence** and **Off sequence** from the command library. Select a device, search its commands, and click a command to append it. Denon inputs and LG inputs/apps are discovered when available. Add delays where a device needs time to start; drag steps or use the arrows to reorder them.
3. Under **Physical buttons**, map short and long presses to included devices. Long Back is reserved for returning to Couch.
4. Under **Remote screen**, choose an included Kodi player or LG TV as the main screen. It supplies artwork/status and default physical controls. Optionally add the activity to areas; it is also listed in its room.

This follows the device selection, sequence, and button mapping workflow described in [Unfolded Circle's activity guide](https://support.unfoldedcircle.com/hc/en-us/articles/13269248303516-How-to-create-an-activity). This version uses Couch's existing Kodi and TV screens; it does not include a custom multipage widget editor.

## Running and ending

Opening an activity runs its On sequence before opening the control screen. Returning to Couch leaves it running; reopening it does not replay On. Tap **End**, or press an unmapped physical Power button, to run Off and return to Couch. An explicit Power mapping takes precedence. **Keep remote awake while running** prevents display standby for the lifetime of that activity, including while viewing another screen.

Commands run in order on a worker, outside the GUI event loop. A failure stops at that step and shows an error. There are no automatic retries or rollback commands. Cancel (or hold Back during a sequence) prevents subsequent steps; an already dispatched command may still finish. Completed commands are never undone automatically. Repeating a failed sequence starts it from the beginning, so prefer explicit on/off commands over toggles.

Running activities and their configuration snapshots are held in memory. GUI restart clears running state; reopening afterward runs On again. Off uses the configuration captured when the activity started. Changing configuration does not alter an already running activity's shutdown sequence.

## Compatibility and limits

`Activity.setup` stores included device IDs, description, keep-awake, and ordered `on`/`off` steps. Each step is a typed command or a delay. Sequences allow 64 steps, delays of 1–30,000 ms, and at most two minutes of total delay each. Commands must be supported by an included device. Removing a device cleans up its sequence commands.

Older `Activity.steps` were saved drafts, not executed commands. They remain visible for reference and are never automatically run. Older API clients omitting `setup` preserve the saved setup. Physical mappings and existing source screens retain their behavior.

Validation: model and daemon regression tests cover membership, command/delay limits, cleanup, legacy updates, and atomic rejection. GUI tests cover ordered dispatch, delays, cancellation, and stopping on failure. Browser coverage exercises create/edit, discovered inputs, reordering, persistence, and mobile layout; an isolated HA100 fixture additionally verified ordered startup, return/reopen without replay, Power-triggered shutdown, restart after End, and long-Back cancellation before a queued command. The fixture used a fake Kodi endpoint and preserved production configuration.
