/* tslint:disable */
/* eslint-disable */

/**
 * JS-facing engine handle. Construct with `new WasmMatchEngine(xmlString)`.
 */
export class WasmMatchEngine {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Away team's display name.
     */
    get_away_team_name(): string;
    /**
     * Competition id the match belongs to.
     */
    get_competition_id(): string;
    /**
     * Current period (1 = first half, 2 = second half, …).
     */
    get_current_period(): number;
    /**
     * Total match duration in seconds.
     */
    get_duration_secs(): number;
    /**
     * Number of floats written per entity (6).
     */
    get_entity_stride(): number;
    /**
     * Monotonic sequence number of the newest executed event.
     */
    get_event_seq(): bigint;
    /**
     * Kick-off date as recorded by the feed.
     */
    get_game_date(): string;
    /**
     * Home team's display name.
     */
    get_home_team_name(): string;
    /**
     * True once the match has played out.
     */
    get_is_finished(): boolean;
    /**
     * True when player `i` has been sent off (render the walk-off, but
     * keep him out of shape geometry).
     */
    get_is_player_dismissed(i: number): boolean;
    /**
     * Origin x of the most recently dispatched event.
     */
    get_last_event_origin_x(): number;
    /**
     * Origin y of the most recently dispatched event.
     */
    get_last_event_origin_y(): number;
    /**
     * Team index (0 home, 1 away) of the most recently dispatched event.
     */
    get_last_event_team(): number;
    /**
     * Type id of the most recently dispatched event.
     */
    get_last_event_type(): number;
    /**
     * Opta match id.
     */
    get_match_id(): string;
    /**
     * Offside line x (metres) for the team in possession.
     */
    get_offside_line(): number;
    /**
     * Number of actual player entities (normally 22).
     */
    get_player_count(): number;
    /**
     * Duty code of player `i` (0 unknown … 8 striker).
     */
    get_player_duty(i: number): number;
    /**
     * Agent index for a player id, or -1 when unknown.
     */
    get_player_index_by_id(id: string): number;
    /**
     * Intent code of player `i` (0 hold … 6 run).
     */
    get_player_intent(i: number): number;
    /**
     * Opponent agent index marked by player `i` (-1 if none).
     */
    get_player_mark(i: number): number;
    /**
     * Coarse role of player `i`:
     * `0 unknown, 1 goalkeeper, 2 defender, 3 midfielder, 4 forward`.
     */
    get_player_role(i: number): number;
    /**
     * Current speed of player `i` in metres per second.
     */
    get_player_speed(i: number): number;
    /**
     * Velocity `[vx, vy]` m/s of player `i`.
     */
    get_player_velocity(i: number): Float32Array;
    /**
     * Home possession share, 0..1.
     */
    get_possession_home(): number;
    /**
     * Team currently in possession: `0` home, `1` away, `-1` neutral.
     */
    get_possession_team(): number;
    /**
     * Number of entries in the recent-event feed (newest last).
     */
    get_recent_event_count(): number;
    /**
     * Event-type name of feed entry `i` (e.g. "Goal", "Card").
     */
    get_recent_event_label(i: number): string;
    /**
     * Whether feed entry `i` deserves a large splash.
     */
    get_recent_event_notable(i: number): boolean;
    /**
     * Player id credited on feed entry `i` (empty for team rows).
     */
    get_recent_event_player(i: number): string;
    /**
     * Team index of feed entry `i` (0 home, 1 away, 2 neutral).
     */
    get_recent_event_team(i: number): number;
    /**
     * Match clock of feed entry `i` (seconds).
     */
    get_recent_event_time(i: number): number;
    /**
     * Event type id of feed entry `i`.
     */
    get_recent_event_type(i: number): number;
    /**
     * A *copy* of the flat render buffer (handy for tests/debugging where
     * raw linear-memory access is unavailable).
     */
    get_render_buffer_copy(): Float32Array;
    /**
     * Pointer to the flat `f32` render buffer in WASM linear memory.
     */
    get_render_buffer_ptr(): number;
    /**
     * Away goals.
     */
    get_score_away(): number;
    /**
     * Home goals.
     */
    get_score_home(): number;
    /**
     * Season label.
     */
    get_season(): string;
    /**
     * True while a set piece is reshaping both teams.
     */
    get_set_piece_active(): boolean;
    /**
     * Active set-piece kind, or empty during open play.
     */
    get_set_piece_kind(): string;
    /**
     * Tournament clock in seconds.
     */
    get_sim_time(): number;
    /**
     * Cover agent index for team `t` (-1 if none).
     */
    get_team_cover(t: number): number;
    /**
     * Phase code for team `t` (0 build-up … 7 defensive transition).
     */
    get_team_phase(t: number): number;
    /**
     * Presser / cover / runner agent index for team `t` (-1 if none).
     */
    get_team_presser(t: number): number;
    /**
     * Runner agent index for team `t` (-1 if none).
     */
    get_team_runner(t: number): number;
    /**
     * Number of entity slots (23).
     */
    get_total_entities(): number;
    /**
     * Build from an Opta F24 XML string.
     */
    constructor(xml_content: string);
    /**
     * Jump to `secs` seconds of match time (0 = kick-off).
     */
    seek_to(secs: number): boolean;
    /**
     * Advance by the engine's fixed 60 Hz timestep.
     */
    step(): void;
    /**
     * Advance physics and choreography by `dt` seconds (0.01667 for 60 Hz).
     */
    tick(dt: number): void;
}
