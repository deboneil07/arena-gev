/* @ts-self-types="./match_engine.d.ts" */

/**
 * JS-facing engine handle. Construct with `new WasmMatchEngine(xmlString)`.
 */
class WasmMatchEngine {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmMatchEngineFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmmatchengine_free(ptr, 0);
    }
    /**
     * Away team's display name.
     * @returns {string}
     */
    get_away_team_name() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.wasmmatchengine_get_away_team_name(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Competition id the match belongs to.
     * @returns {string}
     */
    get_competition_id() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.wasmmatchengine_get_competition_id(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Current period (1 = first half, 2 = second half, …).
     * @returns {number}
     */
    get_current_period() {
        const ret = wasm.wasmmatchengine_get_current_period(this.__wbg_ptr);
        return ret;
    }
    /**
     * Total match duration in seconds.
     * @returns {number}
     */
    get_duration_secs() {
        const ret = wasm.wasmmatchengine_get_duration_secs(this.__wbg_ptr);
        return ret;
    }
    /**
     * Number of floats written per entity (6).
     * @returns {number}
     */
    get_entity_stride() {
        const ret = wasm.wasmmatchengine_get_entity_stride(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Monotonic sequence number of the newest executed event.
     * @returns {bigint}
     */
    get_event_seq() {
        const ret = wasm.wasmmatchengine_get_event_seq(this.__wbg_ptr);
        return BigInt.asUintN(64, ret);
    }
    /**
     * Kick-off date as recorded by the feed.
     * @returns {string}
     */
    get_game_date() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.wasmmatchengine_get_game_date(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Home team's display name.
     * @returns {string}
     */
    get_home_team_name() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.wasmmatchengine_get_home_team_name(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * True once the match has played out.
     * @returns {boolean}
     */
    get_is_finished() {
        const ret = wasm.wasmmatchengine_get_is_finished(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * True when player `i` has been sent off (render the walk-off, but
     * keep him out of shape geometry).
     * @param {number} i
     * @returns {boolean}
     */
    get_is_player_dismissed(i) {
        const ret = wasm.wasmmatchengine_get_is_player_dismissed(this.__wbg_ptr, i);
        return ret !== 0;
    }
    /**
     * Origin x of the most recently dispatched event.
     * @returns {number}
     */
    get_last_event_origin_x() {
        const ret = wasm.wasmmatchengine_get_last_event_origin_x(this.__wbg_ptr);
        return ret;
    }
    /**
     * Origin y of the most recently dispatched event.
     * @returns {number}
     */
    get_last_event_origin_y() {
        const ret = wasm.wasmmatchengine_get_last_event_origin_y(this.__wbg_ptr);
        return ret;
    }
    /**
     * Team index (0 home, 1 away) of the most recently dispatched event.
     * @returns {number}
     */
    get_last_event_team() {
        const ret = wasm.wasmmatchengine_get_last_event_team(this.__wbg_ptr);
        return ret;
    }
    /**
     * Type id of the most recently dispatched event.
     * @returns {number}
     */
    get_last_event_type() {
        const ret = wasm.wasmmatchengine_get_last_event_type(this.__wbg_ptr);
        return ret;
    }
    /**
     * Opta match id.
     * @returns {string}
     */
    get_match_id() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.wasmmatchengine_get_match_id(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Offside line x (metres) for the team in possession.
     * @returns {number}
     */
    get_offside_line() {
        const ret = wasm.wasmmatchengine_get_offside_line(this.__wbg_ptr);
        return ret;
    }
    /**
     * Number of actual player entities (normally 22).
     * @returns {number}
     */
    get_player_count() {
        const ret = wasm.wasmmatchengine_get_player_count(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Duty code of player `i` (0 unknown … 8 striker).
     * @param {number} i
     * @returns {number}
     */
    get_player_duty(i) {
        const ret = wasm.wasmmatchengine_get_player_duty(this.__wbg_ptr, i);
        return ret;
    }
    /**
     * Agent index for a player id, or -1 when unknown.
     * @param {string} id
     * @returns {number}
     */
    get_player_index_by_id(id) {
        const ptr0 = passStringToWasm0(id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmmatchengine_get_player_index_by_id(this.__wbg_ptr, ptr0, len0);
        return ret;
    }
    /**
     * Intent code of player `i` (0 hold … 6 run).
     * @param {number} i
     * @returns {number}
     */
    get_player_intent(i) {
        const ret = wasm.wasmmatchengine_get_player_intent(this.__wbg_ptr, i);
        return ret;
    }
    /**
     * Opponent agent index marked by player `i` (-1 if none).
     * @param {number} i
     * @returns {number}
     */
    get_player_mark(i) {
        const ret = wasm.wasmmatchengine_get_player_mark(this.__wbg_ptr, i);
        return ret;
    }
    /**
     * Coarse role of player `i`:
     * `0 unknown, 1 goalkeeper, 2 defender, 3 midfielder, 4 forward`.
     * @param {number} i
     * @returns {number}
     */
    get_player_role(i) {
        const ret = wasm.wasmmatchengine_get_player_role(this.__wbg_ptr, i);
        return ret;
    }
    /**
     * Current speed of player `i` in metres per second.
     * @param {number} i
     * @returns {number}
     */
    get_player_speed(i) {
        const ret = wasm.wasmmatchengine_get_player_speed(this.__wbg_ptr, i);
        return ret;
    }
    /**
     * Velocity `[vx, vy]` m/s of player `i`.
     * @param {number} i
     * @returns {Float32Array}
     */
    get_player_velocity(i) {
        const ret = wasm.wasmmatchengine_get_player_velocity(this.__wbg_ptr, i);
        var v1 = getArrayF32FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * Home possession share, 0..1.
     * @returns {number}
     */
    get_possession_home() {
        const ret = wasm.wasmmatchengine_get_possession_home(this.__wbg_ptr);
        return ret;
    }
    /**
     * Team currently in possession: `0` home, `1` away, `-1` neutral.
     * @returns {number}
     */
    get_possession_team() {
        const ret = wasm.wasmmatchengine_get_possession_team(this.__wbg_ptr);
        return ret;
    }
    /**
     * Number of entries in the recent-event feed (newest last).
     * @returns {number}
     */
    get_recent_event_count() {
        const ret = wasm.wasmmatchengine_get_recent_event_count(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Event-type name of feed entry `i` (e.g. "Goal", "Card").
     * @param {number} i
     * @returns {string}
     */
    get_recent_event_label(i) {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.wasmmatchengine_get_recent_event_label(this.__wbg_ptr, i);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Whether feed entry `i` deserves a large splash.
     * @param {number} i
     * @returns {boolean}
     */
    get_recent_event_notable(i) {
        const ret = wasm.wasmmatchengine_get_recent_event_notable(this.__wbg_ptr, i);
        return ret !== 0;
    }
    /**
     * Player id credited on feed entry `i` (empty for team rows).
     * @param {number} i
     * @returns {string}
     */
    get_recent_event_player(i) {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.wasmmatchengine_get_recent_event_player(this.__wbg_ptr, i);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Team index of feed entry `i` (0 home, 1 away, 2 neutral).
     * @param {number} i
     * @returns {number}
     */
    get_recent_event_team(i) {
        const ret = wasm.wasmmatchengine_get_recent_event_team(this.__wbg_ptr, i);
        return ret;
    }
    /**
     * Match clock of feed entry `i` (seconds).
     * @param {number} i
     * @returns {number}
     */
    get_recent_event_time(i) {
        const ret = wasm.wasmmatchengine_get_recent_event_time(this.__wbg_ptr, i);
        return ret;
    }
    /**
     * Event type id of feed entry `i`.
     * @param {number} i
     * @returns {number}
     */
    get_recent_event_type(i) {
        const ret = wasm.wasmmatchengine_get_recent_event_type(this.__wbg_ptr, i);
        return ret;
    }
    /**
     * A *copy* of the flat render buffer (handy for tests/debugging where
     * raw linear-memory access is unavailable).
     * @returns {Float32Array}
     */
    get_render_buffer_copy() {
        const ret = wasm.wasmmatchengine_get_render_buffer_copy(this.__wbg_ptr);
        var v1 = getArrayF32FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * Pointer to the flat `f32` render buffer in WASM linear memory.
     * @returns {number}
     */
    get_render_buffer_ptr() {
        const ret = wasm.wasmmatchengine_get_render_buffer_ptr(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Away goals.
     * @returns {number}
     */
    get_score_away() {
        const ret = wasm.wasmmatchengine_get_score_away(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Home goals.
     * @returns {number}
     */
    get_score_home() {
        const ret = wasm.wasmmatchengine_get_score_home(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Season label.
     * @returns {string}
     */
    get_season() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.wasmmatchengine_get_season(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * True while a set piece is reshaping both teams.
     * @returns {boolean}
     */
    get_set_piece_active() {
        const ret = wasm.wasmmatchengine_get_set_piece_active(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * Active set-piece kind, or empty during open play.
     * @returns {string}
     */
    get_set_piece_kind() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.wasmmatchengine_get_set_piece_kind(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Tournament clock in seconds.
     * @returns {number}
     */
    get_sim_time() {
        const ret = wasm.wasmmatchengine_get_sim_time(this.__wbg_ptr);
        return ret;
    }
    /**
     * Cover agent index for team `t` (-1 if none).
     * @param {number} t
     * @returns {number}
     */
    get_team_cover(t) {
        const ret = wasm.wasmmatchengine_get_team_cover(this.__wbg_ptr, t);
        return ret;
    }
    /**
     * Phase code for team `t` (0 build-up … 7 defensive transition).
     * @param {number} t
     * @returns {number}
     */
    get_team_phase(t) {
        const ret = wasm.wasmmatchengine_get_team_phase(this.__wbg_ptr, t);
        return ret;
    }
    /**
     * Presser / cover / runner agent index for team `t` (-1 if none).
     * @param {number} t
     * @returns {number}
     */
    get_team_presser(t) {
        const ret = wasm.wasmmatchengine_get_team_presser(this.__wbg_ptr, t);
        return ret;
    }
    /**
     * Runner agent index for team `t` (-1 if none).
     * @param {number} t
     * @returns {number}
     */
    get_team_runner(t) {
        const ret = wasm.wasmmatchengine_get_team_runner(this.__wbg_ptr, t);
        return ret;
    }
    /**
     * Number of entity slots (23).
     * @returns {number}
     */
    get_total_entities() {
        const ret = wasm.wasmmatchengine_get_total_entities(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Build from an Opta F24 XML string.
     * @param {string} xml_content
     */
    constructor(xml_content) {
        const ptr0 = passStringToWasm0(xml_content, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmmatchengine_new(ptr0, len0);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        this.__wbg_ptr = ret[0];
        WasmMatchEngineFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * Jump to `secs` seconds of match time (0 = kick-off).
     * @param {number} secs
     * @returns {boolean}
     */
    seek_to(secs) {
        const ret = wasm.wasmmatchengine_seek_to(this.__wbg_ptr, secs);
        return ret !== 0;
    }
    /**
     * Advance by the engine's fixed 60 Hz timestep.
     */
    step() {
        wasm.wasmmatchengine_step(this.__wbg_ptr);
    }
    /**
     * Advance physics and choreography by `dt` seconds (0.01667 for 60 Hz).
     * @param {number} dt
     */
    tick(dt) {
        wasm.wasmmatchengine_tick(this.__wbg_ptr, dt);
    }
}
if (Symbol.dispose) WasmMatchEngine.prototype[Symbol.dispose] = WasmMatchEngine.prototype.free;
exports.WasmMatchEngine = WasmMatchEngine;
function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg___wbindgen_throw_41e9ee4f547fc59a: function(arg0, arg1) {
            throw new Error(getStringFromWasm0(arg0, arg1));
        },
        __wbindgen_generic_0000000000000001: function(arg0, arg1) {
            // Cast intrinsic for `Ref(String) -> Externref`.
            const ret = getStringFromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_init_externref_table: function() {
            const table = wasm.__wbindgen_externrefs;
            const offset = table.grow(4);
            table.set(0, undefined);
            table.set(offset + 0, undefined);
            table.set(offset + 1, null);
            table.set(offset + 2, true);
            table.set(offset + 3, false);
        },
    };
    return {
        __proto__: null,
        "./match_engine_bg.js": import0,
    };
}

const WasmMatchEngineFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmmatchengine_free(ptr, 1));

function getArrayF32FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getFloat32ArrayMemory0().subarray(ptr / 4, ptr / 4 + len);
}

let cachedFloat32ArrayMemory0 = null;
function getFloat32ArrayMemory0() {
    if (cachedFloat32ArrayMemory0 === null || cachedFloat32ArrayMemory0.byteLength === 0) {
        cachedFloat32ArrayMemory0 = new Float32Array(wasm.memory.buffer);
    }
    return cachedFloat32ArrayMemory0;
}

function getStringFromWasm0(ptr, len) {
    return decodeText(ptr >>> 0, len);
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

function takeFromExternrefTable0(idx) {
    const value = wasm.__wbindgen_externrefs.get(idx);
    wasm.__externref_table_dealloc(idx);
    return value;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
cachedTextDecoder.decode();
function decodeText(ptr, len) {
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;

const wasmPath = `${__dirname}/match_engine_bg.wasm`;
const wasmBytes = require('fs').readFileSync(wasmPath);
const wasmModule = new WebAssembly.Module(wasmBytes);
let wasmInstance = new WebAssembly.Instance(wasmModule, __wbg_get_imports());
let wasm = wasmInstance.exports;
wasm.__wbindgen_start();
