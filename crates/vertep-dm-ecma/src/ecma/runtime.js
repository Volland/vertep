// Vertep ECMAScript datamodel runtime.
//
// Evaluated once per session context, before any chart code runs. It captures the intrinsics
// it needs, installs the read-only system variables and returns an object of helpers that is
// held by Rust and never reachable from chart code. Everything here is deterministic.
(function (G) {
  "use strict";
  const Rf = Reflect;
  const apply = Rf.apply;
  const construct = Rf.construct;
  const ownKeys = Rf.ownKeys;
  const getDesc = Rf.getOwnPropertyDescriptor;
  const defineProperty = Rf.defineProperty;
  const getProto = Rf.getPrototypeOf;
  const setProto = Rf.setPrototypeOf;
  const rHas = Rf.has;
  const rGet = Rf.get;
  const isExtensible = Rf.isExtensible;
  const preventExtensions = Rf.preventExtensions;
  const deleteProperty = Rf.deleteProperty;
  const freeze = Object.freeze;
  const isFrozen = Object.isFrozen;
  const objectKeys = Object.keys;
  const isArray = Array.isArray;
  const ProxyC = Proxy;
  const WeakMapC = WeakMap;
  const MapC = Map;
  const SetC = Set;
  const DateC = Date;
  const RegExpC = RegExp;
  const TypeErrorC = TypeError;
  const BigIntC = BigInt;
  const wmGet = WeakMap.prototype.get;
  const wmSet = WeakMap.prototype.set;
  const wmHas = WeakMap.prototype.has;
  const mapGet = Map.prototype.get;
  const mapSet = Map.prototype.set;
  const mapHas = Map.prototype.has;
  const mapForEach = Map.prototype.forEach;
  const setAdd = Set.prototype.add;
  const setHas = Set.prototype.has;
  const setForEach = Set.prototype.forEach;
  const dateGetTime = Date.prototype.getTime;
  const reSource = getDesc(RegExp.prototype, "source").get;
  const reFlags = getDesc(RegExp.prototype, "flags").get;
  const fnToString = Function.prototype.toString;
  const bigintToString = BigInt.prototype.toString;
  const strIndexOf = String.prototype.indexOf;
  const jsonParse = JSON.parse;
  const indirectEval = G.eval;
  const unscopables = Symbol.unscopables;
  const typedArrayLength = getDesc(getProto(Uint8Array.prototype), "length").get;
  const arrayBufferByteLength = getDesc(ArrayBuffer.prototype, "byteLength").get;

  // Sources of nondeterminism that depend on garbage collection are removed.
  deleteProperty(G, "WeakRef");
  deleteProperty(G, "FinalizationRegistry");

  function isObj(v) {
    return v !== null && (typeof v === "object" || typeof v === "function");
  }
  function push(a, v) {
    defineProperty(a, a.length, { value: v, writable: true, enumerable: true, configurable: true });
  }
  function put(a, i, v) {
    defineProperty(a, i, { value: v, writable: true, enumerable: true, configurable: true });
  }
  function brand(fn, o, args) {
    try {
      apply(fn, o, args);
      return true;
    } catch (e) {
      return false;
    }
  }

  // ---- System variables (W3C SCXML 5.10) -------------------------------------------------
  const sys = { __proto__: null };
  const sysNames = ["_event", "_sessionid", "_name", "_ioprocessors"];
  for (let i = 0; i < sysNames.length; i++) {
    const k = sysNames[i];
    sys[k] = undefined;
    defineProperty(G, k, {
      get: function () {
        return sys[k];
      },
      set: function (v) {
        throw new TypeErrorC("cannot assign to the read-only system variable " + k);
      },
      enumerable: false,
      configurable: false,
    });
  }
  function deepFreeze(v) {
    if (isObj(v) && !isFrozen(v)) {
      freeze(v);
      const ks = ownKeys(v);
      for (let i = 0; i < ks.length; i++) {
        const d = getDesc(v, ks[i]);
        if (d !== undefined && "value" in d) deepFreeze(d.value);
      }
    }
    return v;
  }
  function setSys(name, value) {
    sys[name] = deepFreeze(value);
  }

  // ---- Global lexical bindings created by <script> (let, const, class) ---------------------
  const lexNames = new MapC();
  function addLexical(name) {
    apply(mapSet, lexNames, [name, true]);
  }
  function isLexical(name) {
    return typeof name === "string" && apply(mapHas, lexNames, [name]);
  }
  function readLexical(name) {
    return indirectEval(name);
  }

  // ---- Read-only view of the session scope for conditions (guard purity) -----------------
  // Builtins whose receiver is unwrapped because they need internal slots and cannot mutate.
  const pureMethods = new WeakMapC();
  function markPure(proto, names) {
    for (let i = 0; i < names.length; i++) {
      const d = getDesc(proto, names[i]);
      if (d !== undefined && typeof d.value === "function") apply(wmSet, pureMethods, [d.value, true]);
    }
  }
  {
    const dp = Date.prototype;
    const ks = ownKeys(dp);
    const names = [];
    for (let i = 0; i < ks.length; i++) {
      const k = ks[i];
      if (typeof k === "string" && (apply(strIndexOf, k, ["get"]) === 0 || apply(strIndexOf, k, ["to"]) === 0 || k === "valueOf")) push(names, k);
    }
    markPure(dp, names);
    markPure(Map.prototype, ["get", "has"]);
    markPure(Set.prototype, ["has"]);
    markPure(Number.prototype, ["valueOf", "toString", "toFixed", "toPrecision", "toExponential"]);
    markPure(String.prototype, ["valueOf", "toString"]);
    markPure(Boolean.prototype, ["valueOf", "toString"]);
    markPure(RegExp.prototype, ["test", "toString"]);
  }
  function denyMutation() {
    throw new TypeErrorC("a condition must not modify the datamodel");
  }
  // Each datamodel object is seen through a proxy over an empty "shadow" target, so the proxy
  // invariants (checked against the shadow) never force a raw object out of the view.
  function guardScope() {
    const proxies = new WeakMapC(); // real -> proxy
    const reals = new WeakMapC(); // shadow or proxy -> real
    function wrap(v) {
      if (!isObj(v)) return v;
      let p = apply(wmGet, proxies, [v]);
      if (p !== undefined) return p;
      const shadow = isArray(v) ? [] : typeof v === "function" ? function () {} : {};
      p = new ProxyC(shadow, handler);
      apply(wmSet, proxies, [v, p]);
      apply(wmSet, reals, [shadow, v]);
      apply(wmSet, reals, [p, v]);
      return p;
    }
    function real(t) {
      return apply(wmGet, reals, [t]);
    }
    function unwrap(v) {
      if (!isObj(v)) return v;
      const t = apply(wmGet, reals, [v]);
      return t === undefined ? v : t;
    }
    const handler = {
      __proto__: null,
      get(t, k, r) {
        const o = real(t);
        const slotted = k === "size" && (brand(mapHas, o, [undefined]) || brand(setHas, o, [undefined]));
        return wrap(rGet(o, k, slotted ? o : r));
      },
      has(t, k) {
        return rHas(real(t), k);
      },
      ownKeys(t) {
        const ks = ownKeys(real(t));
        const own = ownKeys(t);
        for (let i = 0; i < own.length; i++) {
          const d = getDesc(t, own[i]);
          let present = false;
          for (let j = 0; j < ks.length; j++) if (ks[j] === own[i]) present = true;
          if (!d.configurable && !present) push(ks, own[i]);
        }
        return ks;
      },
      getOwnPropertyDescriptor(t, k) {
        const d = getDesc(real(t), k);
        const sd = getDesc(t, k);
        if (d === undefined) return sd !== undefined && !sd.configurable ? sd : undefined;
        if ("value" in d) {
          d.value = wrap(d.value);
          if (sd !== undefined && !sd.configurable) d.writable = true;
        } else {
          d.get = wrap(d.get);
          d.set = wrap(d.set);
        }
        d.configurable = sd !== undefined && !sd.configurable ? false : true;
        return d;
      },
      getPrototypeOf(t) {
        return wrap(getProto(real(t)));
      },
      apply(t, self, args) {
        const f = real(t);
        const receiver = apply(wmHas, pureMethods, [f]) ? unwrap(self) : self;
        return wrap(apply(f, receiver, args));
      },
      construct(t, args, nt) {
        // A constructed object is new, not part of the datamodel, so it is not wrapped.
        return construct(real(t), args, unwrap(nt));
      },
      set: denyMutation,
      defineProperty: denyMutation,
      deleteProperty: denyMutation,
      setPrototypeOf: denyMutation,
      preventExtensions: denyMutation,
    };
    // The scope object seen by `with`: global properties and global lexical bindings.
    const scopeHandler = {
      __proto__: null,
      has(t, k) {
        if (k === unscopables) return false;
        return rHas(G, k) || isLexical(k);
      },
      get(t, k, r) {
        if (k === unscopables) return undefined;
        if (!rHas(G, k) && isLexical(k)) return wrap(readLexical(k));
        return wrap(rGet(G, k, r));
      },
      set: denyMutation,
      defineProperty: denyMutation,
      deleteProperty: denyMutation,
    };
    return new ProxyC({ __proto__: null }, scopeHandler);
  }

  // ---- Classes: boa keeps no source text for class constructors ---------------------------
  const classSources = new WeakMapC();
  // Class members re-created from regenerated class source, mapped to their original text.
  const sourceOverrides = new WeakMapC();
  function registerClass(name, src) {
    let ctor;
    try {
      ctor = indirectEval(name);
    } catch (e) {
      return false;
    }
    if (typeof ctor !== "function") return false;
    apply(wmSet, classSources, [ctor, src]);
    return true;
  }

  // ---- Global bookkeeping -----------------------------------------------------------------
  function globalKeys() {
    return ownKeys(G);
  }
  // Deletes global properties created since `before` was taken; returns how many.
  function removeNewKeys(before) {
    const now = ownKeys(G);
    if (now.length === before.length) return 0;
    const old = new SetC();
    for (let i = 0; i < before.length; i++) apply(setAdd, old, [before[i]]);
    let removed = 0;
    for (let i = 0; i < now.length; i++) {
      if (!apply(setHas, old, [now[i]])) {
        deleteProperty(G, now[i]);
        removed++;
      }
    }
    return removed;
  }

  // ---- Intrinsics, referenced by name in snapshots -----------------------------------------
  const intrinsicName = new MapC();
  const intrinsicByName = new MapC();
  function addIntrinsic(name, v) {
    if (isObj(v) && !apply(mapHas, intrinsicName, [v]) && !apply(mapHas, intrinsicByName, [name])) {
      apply(mapSet, intrinsicName, [v, name]);
      apply(mapSet, intrinsicByName, [name, v]);
    }
  }
  function addMembers(prefix, o, depth) {
    const ks = ownKeys(o);
    for (let i = 0; i < ks.length; i++) {
      const k = ks[i];
      if (typeof k !== "string") continue;
      const d = getDesc(o, k);
      if (d === undefined) continue;
      const name = prefix + "." + k;
      if ("value" in d) {
        if (isObj(d.value)) {
          const known = apply(mapHas, intrinsicName, [d.value]);
          addIntrinsic(name, d.value);
          if (!known && depth > 0) addMembers(name, d.value, depth - 1);
        }
      } else {
        addIntrinsic(name + "#get", d.get);
        addIntrinsic(name + "#set", d.set);
      }
    }
  }
  addIntrinsic("globalThis", G);
  addMembers("globalThis", G, 2);
  addIntrinsic("%GeneratorFunction.prototype%", getProto(function* () {}));
  addIntrinsic("%AsyncFunction.prototype%", getProto(async function () {}));
  addIntrinsic("%AsyncGeneratorFunction.prototype%", getProto(async function* () {}));
  addMembers("%GeneratorFunction.prototype%", getProto(function* () {}), 1);
  addMembers("%AsyncGeneratorFunction.prototype%", getProto(async function* () {}), 1);

  // ---- Baseline: global names that exist before any chart code runs -----------------------
  const baseline = new SetC();
  function sealBaseline() {
    const ks = ownKeys(G);
    for (let i = 0; i < ks.length; i++) apply(setAdd, baseline, [ks[i]]);
  }
  // Global names that belong to the datamodel: created by the chart, or declared over a baseline name.
  function userGlobalNames(declared) {
    const out = [];
    const extra = new SetC();
    for (let i = 0; i < declared.length; i++) apply(setAdd, extra, [declared[i]]);
    const ks = ownKeys(G);
    for (let i = 0; i < ks.length; i++) {
      const k = ks[i];
      if (typeof k === "string" && (!apply(setHas, baseline, [k]) || apply(setHas, extra, [k]))) push(out, k);
    }
    return out;
  }

  // ---- Snapshot encoding --------------------------------------------------------------------
  function flagsOf(d) {
    return (d.writable ? "w" : "") + (d.enumerable ? "e" : "") + (d.configurable ? "c" : "");
  }
  function encodeState(globalNames, lexical) {
    const ids = new MapC();
    const heap = [];
    const queue = [];
    function enc(v) {
      switch (typeof v) {
        case "undefined":
        case "boolean":
        case "number":
        case "string":
          return v;
        case "bigint":
          return { __proto__: null, bigint: apply(bigintToString, v, []) };
        case "symbol":
          return { __proto__: null, unsupported: "symbol" };
      }
      if (v === null) return null;
      const name = apply(mapGet, intrinsicName, [v]);
      if (name !== undefined) return { __proto__: null, intrinsic: name };
      let id = apply(mapGet, ids, [v]);
      if (id === undefined) {
        id = heap.length;
        apply(mapSet, ids, [v, id]);
        push(heap, null);
        push(queue, v);
      }
      return { __proto__: null, ref: id };
    }
    function encProps(o) {
      const out = [];
      const ks = ownKeys(o);
      for (let i = 0; i < ks.length; i++) {
        const k = ks[i];
        if (typeof k !== "string") continue;
        const d = getDesc(o, k);
        if ("value" in d) push(out, [k, enc(d.value), flagsOf(d)]);
        else push(out, [k, enc(d.get), flagsOf(d), enc(d.set)]);
      }
      return out;
    }
    function entry(o) {
      const e = { __proto__: null, kind: "object" };
      if (typeof o === "function") {
        const classSrc = apply(wmGet, classSources, [o]);
        const override = apply(wmGet, sourceOverrides, [o]);
        const src = classSrc !== undefined ? classSrc : override !== undefined ? override : apply(fnToString, o, []);
        if (classSrc === undefined && apply(strIndexOf, src, ["[native code]"]) >= 0) {
          return { __proto__: null, kind: "unsupported", type: "native, bound or nested-class function" };
        }
        e.kind = classSrc !== undefined ? "class" : "function";
        e.source = src;
        const pd = getDesc(o, "prototype");
        if (pd !== undefined && "value" in pd && isObj(pd.value) && apply(mapGet, intrinsicName, [pd.value]) === undefined) {
          const cd = getDesc(pd.value, "constructor");
          if (cd !== undefined && cd.value === o) e.prototypeObject = enc(pd.value).ref;
        }
      } else if (isArray(o)) {
        e.kind = "array";
      } else if (brand(dateGetTime, o, [])) {
        e.kind = "date";
        e.time = apply(dateGetTime, o, []);
      } else if (brand(reSource, o, []) && o !== RegExp.prototype) {
        e.kind = "regexp";
        e.source = apply(reSource, o, []);
        e.flags = apply(reFlags, o, []);
      } else if (brand(mapHas, o, [undefined])) {
        e.kind = "map";
        const entries = [];
        apply(mapForEach, o, [function (v, k) { push(entries, [enc(k), enc(v)]); }]);
        e.entries = entries;
      } else if (brand(setHas, o, [undefined])) {
        e.kind = "set";
        const values = [];
        apply(setForEach, o, [function (v) { push(values, enc(v)); }]);
        e.values = values;
      } else if (brand(wmHas, o, [{}])) {
        return { __proto__: null, kind: "unsupported", type: "WeakMap" };
      } else if (brand(typedArrayLength, o, [])) {
        return { __proto__: null, kind: "unsupported", type: "typed array" };
      } else if (brand(arrayBufferByteLength, o, [])) {
        return { __proto__: null, kind: "unsupported", type: "ArrayBuffer" };
      }
      e.proto = enc(getProto(o));
      e.extensible = isExtensible(o);
      e.props = encProps(o);
      return e;
    }
    const globals = [];
    for (let i = 0; i < globalNames.length; i++) {
      const name = globalNames[i];
      const d = getDesc(G, name);
      if (d === undefined) continue;
      if ("value" in d) push(globals, [name, enc(d.value), flagsOf(d)]);
      else push(globals, [name, enc(d.get), flagsOf(d), enc(d.set)]);
    }
    const lex = [];
    for (let i = 0; i < lexical.length; i++) {
      push(lex, [lexical[i][0], enc(readLexical(lexical[i][0])), lexical[i][1]]);
    }
    for (let i = 0; i < queue.length; i++) put(heap, i, entry(queue[i]));
    return { __proto__: null, globals: globals, lexical: lex, heap: heap };
  }

  // ---- Snapshot decoding (runs on a fresh context, before any chart code) -----------------
  let heapIn = [];
  let objs = [];
  let creating = [];
  // While decoding, names of the snapshot's globals resolve to their decoded values, so that a
  // class's `extends` clause finds its superclass before the globals are defined.
  let restoring = null;
  const restoreScope = new ProxyC({ __proto__: null }, {
    __proto__: null,
    has(t, k) {
      return restoring !== null && typeof k === "string" && apply(mapHas, restoring, [k]);
    },
    get(t, k) {
      if (k === unscopables || restoring === null) return undefined;
      return dec(apply(mapGet, restoring, [k]));
    },
  });
  const makeClass = indirectEval(
    "(function (__vertepRestoreScope__) { with (__vertepRestoreScope__) { " +
      "return function () { return eval('(' + arguments[0] + '\\n)'); }; } })",
  )(restoreScope);
  function makeFunction(src) {
    try {
      return indirectEval("(" + src + "\n)");
    } catch (err) {
      // Methods (`f() {}`, `async *g() {}`) are not expressions on their own.
      const holder = indirectEval("({" + src + "\n})");
      const ks = ownKeys(holder);
      const d = getDesc(holder, ks[0]);
      return "value" in d ? d.value : d.get !== undefined ? d.get : d.set;
    }
  }
  function shell(i) {
    if (objs[i] !== undefined) return objs[i];
    if (creating[i]) throw new TypeErrorC("snapshot has a cyclic class hierarchy");
    creating[i] = true;
    const e = heapIn[i];
    let o;
    switch (e.kind) {
      case "class":
        o = makeClass(e.source);
        apply(wmSet, classSources, [o, e.source]);
        break;
      case "function": o = makeFunction(e.source); break;
      case "object": o = {}; break;
      case "array": o = []; break;
      case "date": o = new DateC(e.time); break;
      case "regexp": o = new RegExpC(e.source, e.flags); break;
      case "map": o = new MapC(); break;
      case "set": o = new SetC(); break;
      case "unsupported": throw new TypeErrorC("snapshot holds a value that cannot be restored: " + e.type);
      default: throw new TypeErrorC("snapshot has an unknown object kind " + e.kind);
    }
    objs[i] = o;
    if (e.prototypeObject !== undefined && objs[e.prototypeObject] === undefined) {
      objs[e.prototypeObject] = o.prototype;
      if (e.kind === "class") adoptMembers(heapIn[e.prototypeObject], o.prototype);
    }
    if (e.kind === "class") adoptMembers(e, o);
    return o;
  }
  // A re-created class already has its methods, with the right home object for `super`:
  // map the snapshot's method entries onto them instead of re-creating them one by one.
  function adoptMembers(e, o) {
    for (let j = 0; j < e.props.length; j++) {
      const p = e.props[j];
      const fresh = getDesc(o, p[0]);
      if (fresh === undefined) continue;
      if (p.length === 4) {
        adopt(p[1], fresh.get);
        adopt(p[3], fresh.set);
      } else if ("value" in fresh) {
        adopt(p[1], fresh.value);
      }
    }
  }
  function adopt(encoded, fresh) {
    if (isObj(encoded) && "ref" in encoded && typeof fresh === "function") {
      const idx = encoded.ref;
      if (idx >= 0 && idx < heapIn.length && heapIn[idx].kind === "function" && objs[idx] === undefined) {
        objs[idx] = fresh;
        // Keep the original text so the next snapshot is identical to this one.
        apply(wmSet, sourceOverrides, [fresh, heapIn[idx].source]);
      }
    }
  }
  function dec(e) {
    if (!isObj(e)) return e;
    if ("ref" in e) {
      if (!(e.ref >= 0 && e.ref < heapIn.length)) throw new TypeErrorC("snapshot reference out of range");
      return shell(e.ref);
    }
    if ("intrinsic" in e) {
      const v = apply(mapGet, intrinsicByName, [e.intrinsic]);
      if (v === undefined) throw new TypeErrorC("snapshot names an unknown intrinsic " + e.intrinsic);
      return v;
    }
    if ("bigint" in e) return BigIntC(e.bigint);
    if ("unsupported" in e) throw new TypeErrorC("snapshot holds a value that cannot be restored: " + e.unsupported);
    throw new TypeErrorC("malformed snapshot value");
  }
  function descriptor(p) {
    const f = p[2];
    const d = { __proto__: null, enumerable: apply(strIndexOf, f, ["e"]) >= 0, configurable: apply(strIndexOf, f, ["c"]) >= 0 };
    if (p.length === 4) {
      d.get = dec(p[1]);
      d.set = dec(p[3]);
    } else {
      d.value = dec(p[1]);
      d.writable = apply(strIndexOf, f, ["w"]) >= 0;
    }
    return d;
  }
  // `bindings` lists [name, encoded value] for every global and lexical binding.
  function decodeHeap(heap, bindings) {
    heapIn = heap;
    objs = [];
    creating = [];
    restoring = new MapC();
    for (let i = 0; i < bindings.length; i++) apply(mapSet, restoring, [bindings[i][0], bindings[i][1]]);
    try {
      // Functions first: re-creating them also yields their prototypes and class methods.
      for (let i = 0; i < heap.length; i++) if (heap[i].kind === "class" || heap[i].kind === "function") shell(i);
      for (let i = 0; i < heap.length; i++) shell(i);
      for (let i = 0; i < heap.length; i++) {
        const e = heap[i];
        const o = objs[i];
        if (!setProto(o, dec(e.proto))) throw new TypeErrorC("cannot restore a prototype");
        for (let j = 0; j < e.props.length; j++) {
          const p = e.props[j];
          if (!defineProperty(o, p[0], descriptor(p))) throw new TypeErrorC("cannot restore property " + p[0]);
        }
        if (e.kind === "map") for (let j = 0; j < e.entries.length; j++) apply(mapSet, o, [dec(e.entries[j][0]), dec(e.entries[j][1])]);
        if (e.kind === "set") for (let j = 0; j < e.values.length; j++) apply(setAdd, o, [dec(e.values[j])]);
      }
      for (let i = 0; i < heap.length; i++) if (!heap[i].extensible) preventExtensions(objs[i]);
    } finally {
      restoring = null;
    }
  }
  function decodeGlobals(globals) {
    for (let i = 0; i < globals.length; i++) {
      const p = globals[i];
      if (!defineProperty(G, p[0], descriptor(p))) throw new TypeErrorC("cannot restore global " + p[0]);
    }
  }
  // boa resolves a function's free global names when it compiles the function, so functions
  // must be compiled while the restored global lexical bindings exist. Rust therefore runs one
  // script declaring every lexical binding, whose first initializer calls `restoreSlot` and so
  // decodes the heap while all of those bindings are instantiated.
  let pendingDecode = null;
  function prepareDecode(heap, bindings, globals, lexical) {
    pendingDecode = { __proto__: null, heap: heap, bindings: bindings, globals: globals, lexical: lexical, done: false };
  }
  function finishDecode() {
    if (pendingDecode !== null && !pendingDecode.done) {
      pendingDecode.done = true;
      decodeHeap(pendingDecode.heap, pendingDecode.bindings);
      decodeGlobals(pendingDecode.globals);
    }
  }
  function restoreSlot(i) {
    finishDecode();
    return dec(pendingDecode.lexical[i]);
  }
  function endDecode() {
    finishDecode();
    pendingDecode = null;
  }

  // ---- Data helpers ---------------------------------------------------------------------------
  function keys(o) {
    return objectKeys(o);
  }
  function declare(name) {
    const d = getDesc(G, name);
    if (d === undefined) {
      return defineProperty(G, name, { value: undefined, writable: true, enumerable: true, configurable: false });
    }
    if ("value" in d && d.writable) {
      return defineProperty(G, name, { value: undefined });
    }
    if (d.configurable) {
      return defineProperty(G, name, { value: undefined, writable: true, enumerable: true, configurable: false });
    }
    return false;
  }
  function exists(name) {
    return rHas(G, name) || isLexical(name);
  }

  return {
    __proto__: null,
    setSys: setSys,
    guardScope: guardScope,
    globalKeys: globalKeys,
    removeNewKeys: removeNewKeys,
    sealBaseline: sealBaseline,
    userGlobalNames: userGlobalNames,
    encodeState: encodeState,
    prepareDecode: prepareDecode,
    restoreSlot: restoreSlot,
    endDecode: endDecode,
    addLexical: addLexical,
    registerClass: registerClass,
    isArray: isArray,
    keys: keys,
    jsonParse: jsonParse,
    declare: declare,
    exists: exists,
  };
})
