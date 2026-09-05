// Cratebase JSVM prelude: implements the PocketBase globals on top of the
// native bridge `__cb.native(op, args)` installed by bridge.rs.
//
// Everything here runs once per worker context. Handler tables are keyed
// by deterministic ids (`<file>:<kind>:<key>:<ordinal>`), so every worker
// computes the same id for the same source position.

(function (global) {
  const native = __cb.native;
  const hostCall = (op, ...args) => native(op, args);

  // ---------------------------------------------------------------------
  // Errors
  // ---------------------------------------------------------------------

  class ApiError extends Error {
    constructor(status, message, data) {
      super(message == null ? "" : String(message));
      this.name = "ApiError";
      this.status = status || 400;
      this.data = data || {};
      this.isApiError = true;
    }
    toJSON() {
      return { status: this.status, message: this.message, data: this.data };
    }
  }
  const defaultMessages = {
    400: "Something went wrong while processing your request.",
    401: "The request requires valid record authorization token.",
    403: "You are not allowed to perform this request.",
    404: "The requested resource wasn't found.",
    429: "Too Many Requests.",
    500: "Something went wrong while processing your request.",
  };
  function statusError(status, name) {
    const cls = class extends ApiError {
      constructor(message, data) {
        super(status, message || defaultMessages[status], data);
        this.name = name;
      }
    };
    Object.defineProperty(cls, "name", { value: name });
    return cls;
  }
  const BadRequestError = statusError(400, "BadRequestError");
  const UnauthorizedError = statusError(401, "UnauthorizedError");
  const ForbiddenError = statusError(403, "ForbiddenError");
  const NotFoundError = statusError(404, "NotFoundError");
  const TooManyRequestsError = statusError(429, "TooManyRequestsError");
  const InternalServerError = statusError(500, "InternalServerError");
  const byStatus = {
    400: BadRequestError,
    401: UnauthorizedError,
    403: ForbiddenError,
    404: NotFoundError,
    429: TooManyRequestsError,
    500: InternalServerError,
  };
  // Used by the Rust side to throw AppErrors as ApiError instances.
  global.__cbMakeError = (status, message, data) => {
    const cls = byStatus[status];
    return cls ? new cls(message, data) : new ApiError(status, message, data);
  };

  // ---------------------------------------------------------------------
  // Collection
  // ---------------------------------------------------------------------

  const COLLECTION_DEFAULTS = () => ({
    id: "",
    name: "",
    type: "base",
    system: false,
    fields: [],
    indexes: [],
    listRule: null,
    viewRule: null,
    createRule: null,
    updateRule: null,
    deleteRule: null,
    created: "",
    updated: "",
  });

  function fieldsArray(arr) {
    const list = Array.isArray(arr) ? arr : [];
    Object.defineProperty(list, "getByName", {
      value: (name) => list.find((f) => f && f.name === name) || null,
      enumerable: false,
    });
    Object.defineProperty(list, "getById", {
      value: (id) => list.find((f) => f && f.id === id) || null,
      enumerable: false,
    });
    Object.defineProperty(list, "add", {
      value: (...fields) => list.push(...fields),
      enumerable: false,
    });
    Object.defineProperty(list, "removeByName", {
      value: (name) => {
        const i = list.findIndex((f) => f && f.name === name);
        if (i >= 0) list.splice(i, 1);
      },
      enumerable: false,
    });
    return list;
  }

  class Collection {
    constructor(data) {
      const d = Object.assign(COLLECTION_DEFAULTS(), data || {});
      d.fields = fieldsArray(d.fields);
      this.__data = d;
      // Every unknown property reads/writes the underlying JSON, so the
      // flattened auth options (authRule, oauth2, ...) work transparently.
      return new Proxy(this, {
        get(target, prop, receiver) {
          if (prop in target) return Reflect.get(target, prop, receiver);
          return target.__data[prop];
        },
        set(target, prop, value) {
          if (prop === "__data") {
            target.__data = value;
          } else if (prop === "fields") {
            target.__data.fields = fieldsArray(value);
          } else {
            target.__data[prop] = value;
          }
          return true;
        },
        has(target, prop) {
          return prop in target || prop in target.__data;
        },
        ownKeys(target) {
          return Reflect.ownKeys(target.__data);
        },
        getOwnPropertyDescriptor(target, prop) {
          if (prop in target.__data) {
            return { value: target.__data[prop], enumerable: true, configurable: true, writable: true };
          }
          return undefined;
        },
      });
    }
    static __fromJSON(json) {
      return new Collection(json);
    }
    isAuth() {
      return this.__data.type === "auth";
    }
    isBase() {
      return this.__data.type === "base";
    }
    isView() {
      return this.__data.type === "view";
    }
    toJSON() {
      return this.__data;
    }
    __export() {
      return JSON.parse(JSON.stringify(this.__data));
    }
  }

  // ---------------------------------------------------------------------
  // Record
  // ---------------------------------------------------------------------

  const MULTI_TYPES = new Set(["select", "relation", "file"]);
  function zeroValue(field) {
    const multiple = MULTI_TYPES.has(field.type) && (field.maxSelect === undefined || field.maxSelect !== 1);
    if (multiple) return [];
    switch (field.type) {
      case "number":
        return 0;
      case "bool":
        return false;
      case "json":
        return null;
      case "geoPoint":
        return { lon: 0, lat: 0 };
      default:
        return "";
    }
  }

  const HIDDEN_ON_EXPORT = new Set(["password", "tokenKey"]);

  class Record {
    constructor(collection, data) {
      const col = collection instanceof Collection || (collection && collection.__data) ? collection : collection ? new Collection(collection) : null;
      this.__collection = col;
      this.__data = {};
      if (col) {
        for (const f of col.__data.fields) this.__data[f.name] = zeroValue(f);
        this.__data.collectionId = col.__data.id;
        this.__data.collectionName = col.__data.name;
      }
      if (!("id" in this.__data)) this.__data.id = "";
      this.__expand = {};
      this.__isNew = true;
      this.__dirty = new Set();
      this.__original = {};
      if (data) for (const k of Object.keys(data)) this.set(k, data[k]);
    }
    static __fromJSON(json, opts) {
      const r = Object.create(Record.prototype);
      const data = Object.assign({}, json || {});
      const expand = data.expand;
      delete data.expand;
      r.__collection = null;
      r.__data = data;
      r.__expand = expand && typeof expand === "object" ? expand : {};
      r.__isNew = !!(opts && opts.isNew);
      r.__dirty = new Set();
      r.__original = Object.assign({}, data);
      return r;
    }
    get id() {
      return this.__data.id || "";
    }
    set id(v) {
      this.set("id", v);
    }
    get collectionId() {
      return this.__data.collectionId || (this.__collection ? this.__collection.__data.id : "");
    }
    get collectionName() {
      return this.__data.collectionName || (this.__collection ? this.__collection.__data.name : "");
    }
    isNew() {
      return this.__isNew;
    }
    collection() {
      if (!this.__collection) {
        this.__collection = $app.findCollectionByNameOrId(this.collectionId || this.collectionName);
      }
      return this.__collection;
    }
    get(field) {
      const v = this.__data[field];
      return v === undefined ? null : v;
    }
    getRaw(field) {
      return this.get(field);
    }
    set(field, value) {
      if (field === "collectionId" || field === "collectionName") return;
      if (field === "expand") {
        this.__expand = value || {};
        return;
      }
      this.__data[field] = value;
      this.__dirty.add(field);
    }
    load(data) {
      for (const k of Object.keys(data || {})) this.set(k, data[k]);
    }
    getString(field) {
      const v = this.__data[field];
      if (v === undefined || v === null) return "";
      if (typeof v === "string") return v;
      if (Array.isArray(v)) return v.length ? String(v[0]) : "";
      if (typeof v === "object") return JSON.stringify(v);
      return String(v);
    }
    getBool(field) {
      const v = this.__data[field];
      return v === true || v === "true" || v === 1;
    }
    getInt(field) {
      const n = parseInt(this.__data[field], 10);
      return Number.isFinite(n) ? n : 0;
    }
    getFloat(field) {
      const n = parseFloat(this.__data[field]);
      return Number.isFinite(n) ? n : 0;
    }
    getDateTime(field) {
      return new DateTime(this.getString(field));
    }
    getStringSlice(field) {
      const v = this.__data[field];
      if (Array.isArray(v)) return v.map(String);
      if (v === undefined || v === null || v === "") return [];
      return [String(v)];
    }
    getUnsavedFiles(field) {
      const v = this.__data[field];
      const list = Array.isArray(v) ? v : v ? [v] : [];
      return list.filter((x) => x && typeof x === "object" && x.__file);
    }
    original() {
      const r = Record.__fromJSON(this.__original, { isNew: this.__isNew });
      r.__collection = this.__collection;
      return r;
    }
    fresh() {
      return this;
    }
    clone() {
      const r = Record.__fromJSON(this.__export(), { isNew: this.__isNew });
      r.__collection = this.__collection;
      r.__dirty = new Set(this.__dirty);
      return r;
    }
    expand() {
      return this.__expand;
    }
    setExpand(map) {
      this.__expand = map || {};
    }
    mergeExpand(map) {
      Object.assign(this.__expand, map || {});
    }
    expandedOne(rel) {
      const v = this.__expand[rel];
      const first = Array.isArray(v) ? v[0] : v;
      return first ? Record.__fromJSON(first) : null;
    }
    expandedAll(rel) {
      const v = this.__expand[rel];
      const list = Array.isArray(v) ? v : v ? [v] : [];
      return list.map((x) => Record.__fromJSON(x));
    }
    // auth helpers
    email() {
      return this.getString("email");
    }
    setEmail(v) {
      this.set("email", v);
    }
    verified() {
      return this.getBool("verified");
    }
    setVerified(v) {
      this.set("verified", !!v);
    }
    emailVisibility() {
      return this.getBool("emailVisibility");
    }
    setEmailVisibility(v) {
      this.set("emailVisibility", !!v);
    }
    tokenKey() {
      return this.getString("tokenKey");
    }
    setTokenKey(v) {
      this.set("tokenKey", v);
    }
    refreshTokenKey() {
      this.set("tokenKey", $security.randomString(50));
    }
    setPassword(pw) {
      this.set("password", hostCall("hashPassword", String(pw)));
    }
    validatePassword(pw) {
      return hostCall("validatePassword", this.__ref(), String(pw));
    }
    hide(...fields) {
      this.__hidden = (this.__hidden || []).concat(fields);
    }
    unhide(...fields) {
      this.__hidden = (this.__hidden || []).filter((f) => !fields.includes(f));
    }
    publicExport() {
      const out = {};
      const hidden = new Set(this.__hidden || []);
      for (const k of Object.keys(this.__data)) {
        if (HIDDEN_ON_EXPORT.has(k) || hidden.has(k)) continue;
        out[k] = this.__data[k];
      }
      if (Object.keys(this.__expand).length) out.expand = this.__expand;
      return out;
    }
    toJSON() {
      return this.publicExport();
    }
    __export() {
      const out = Object.assign({}, this.__data);
      if (Object.keys(this.__expand).length) out.expand = this.__expand;
      return out;
    }
    __ref() {
      return {
        collection: this.collectionId || this.collectionName,
        id: this.__data.id || "",
        isNew: this.__isNew,
        data: this.__data,
        dirty: [...this.__dirty],
      };
    }
    __absorb(json) {
      const data = Object.assign({}, json || {});
      if (data.expand) this.__expand = data.expand;
      delete data.expand;
      this.__data = data;
      this.__isNew = false;
      this.__dirty = new Set();
      this.__original = Object.assign({}, data);
    }
  }

  function recordFrom(json, opts) {
    return json ? Record.__fromJSON(json, opts) : null;
  }

  function collectionName(c) {
    if (c && (c instanceof Collection || c.__data)) return c.__data.id || c.__data.name;
    return String(c);
  }

  // ---------------------------------------------------------------------
  // DateTime (minimal PocketBase shape)
  // ---------------------------------------------------------------------

  function pad(n, w) {
    return String(n).padStart(w || 2, "0");
  }
  class DateTime {
    constructor(value) {
      this.__date = value === undefined || value === "" || value === null ? new Date() : new Date(value);
    }
    time() {
      return this.__date;
    }
    unix() {
      return Math.floor(this.__date.getTime() / 1000);
    }
    isZero() {
      return isNaN(this.__date.getTime()) || this.__date.getTime() === 0;
    }
    string() {
      if (isNaN(this.__date.getTime())) return "";
      const d = this.__date;
      return (
        d.getUTCFullYear() +
        "-" +
        pad(d.getUTCMonth() + 1) +
        "-" +
        pad(d.getUTCDate()) +
        " " +
        pad(d.getUTCHours()) +
        ":" +
        pad(d.getUTCMinutes()) +
        ":" +
        pad(d.getUTCSeconds()) +
        "." +
        pad(d.getUTCMilliseconds(), 3) +
        "Z"
      );
    }
    toString() {
      return this.string();
    }
    toJSON() {
      return this.string();
    }
  }

  // ---------------------------------------------------------------------
  // $dbx -> filter strings
  // ---------------------------------------------------------------------

  const $dbx = {
    exp: (sql, params) => ({ __dbx: "exp", sql: String(sql), params: params || {} }),
    hashExp: (pairs) => ({ __dbx: "hash", pairs: pairs || {} }),
    and: (...exps) => ({ __dbx: "and", exps }),
    or: (...exps) => ({ __dbx: "or", exps }),
    not: (exp) => ({ __dbx: "not", exp }),
    in: (col, ...values) => ({ __dbx: "in", col, values: values.flat() }),
    notIn: (col, ...values) => ({ __dbx: "notIn", col, values: values.flat() }),
    like: (col, ...values) => ({ __dbx: "like", col, values: values.flat() }),
    notLike: (col, ...values) => ({ __dbx: "notLike", col, values: values.flat() }),
    between: (col, from, to) => ({ __dbx: "between", col, from, to }),
    newExp: (sql, params) => ({ __dbx: "exp", sql: String(sql), params: params || {} }),
  };

  function dbxToFilter(exps) {
    const params = {};
    let counter = 0;
    const bind = (v) => {
      const name = "p" + counter++;
      params[name] = v;
      return "{:" + name + "}";
    };
    const conv = (e) => {
      if (e === undefined || e === null || e === "") return "";
      if (typeof e === "string") return e;
      if (!e.__dbx) throw new BadRequestError("unsupported expression: " + JSON.stringify(e));
      switch (e.__dbx) {
        case "exp": {
          Object.assign(params, e.params);
          return "(" + e.sql + ")";
        }
        case "hash": {
          const parts = Object.keys(e.pairs).map((k) => {
            const v = e.pairs[k];
            if (Array.isArray(v)) {
              if (!v.length) return "false";
              return "(" + v.map((x) => k + " = " + bind(x)).join(" || ") + ")";
            }
            if (v === null) return k + " = ''";
            return k + " = " + bind(v);
          });
          return parts.length ? "(" + parts.join(" && ") + ")" : "";
        }
        case "and": {
          const parts = e.exps.map(conv).filter(Boolean);
          return parts.length ? "(" + parts.join(" && ") + ")" : "";
        }
        case "or": {
          const parts = e.exps.map(conv).filter(Boolean);
          return parts.length ? "(" + parts.join(" || ") + ")" : "";
        }
        case "not": {
          const inner = e.exp;
          if (inner && inner.__dbx === "hash") {
            const parts = Object.keys(inner.pairs).map((k) => k + " != " + bind(inner.pairs[k]));
            return parts.length ? "(" + parts.join(" || ") + ")" : "";
          }
          throw new BadRequestError("$dbx.not supports only hashExp");
        }
        case "in":
        case "notIn": {
          if (!e.values.length) return e.__dbx === "in" ? "false" : "true";
          const op = e.__dbx === "in" ? " = " : " != ";
          const join = e.__dbx === "in" ? " || " : " && ";
          return "(" + e.values.map((v) => e.col + op + bind(v)).join(join) + ")";
        }
        case "like":
        case "notLike": {
          const op = e.__dbx === "like" ? " ~ " : " !~ ";
          return "(" + e.values.map((v) => e.col + op + bind(v)).join(" && ") + ")";
        }
        case "between":
          return "(" + e.col + " >= " + bind(e.from) + " && " + e.col + " <= " + bind(e.to) + ")";
        default:
          throw new BadRequestError("unsupported $dbx expression " + e.__dbx);
      }
    };
    const filter = exps.map(conv).filter(Boolean).join(" && ");
    return { filter, params };
  }

  // ---------------------------------------------------------------------
  // $app
  // ---------------------------------------------------------------------

  const $app = {
    settings: () => hostCall("settings"),
    findCollectionByNameOrId: (nameOrId) => Collection.__fromJSON(hostCall("findCollection", String(nameOrId))),
    findRecordById: (collection, id) => recordFrom(hostCall("findRecordById", collectionName(collection), String(id))),
    findRecordsByFilter: (collection, filter, sort, limit, offset, params) =>
      hostCall(
        "findRecordsByFilter",
        collectionName(collection),
        filter || "",
        sort || "",
        limit || 0,
        offset || 0,
        params || {}
      ).map((r) => recordFrom(r)),
    findFirstRecordByFilter: (collection, filter, params) => {
      const rows = hostCall("findRecordsByFilter", collectionName(collection), filter || "", "", 1, 0, params || {});
      if (!rows.length) throw new NotFoundError("sql: no rows in result set");
      return recordFrom(rows[0]);
    },
    findAllRecords: (collection, ...exps) => {
      const { filter, params } = dbxToFilter(exps);
      return $app.findRecordsByFilter(collection, filter, "", 0, 0, params);
    },
    findFirstRecordByData: (collection, key, value) =>
      $app.findFirstRecordByFilter(collection, key + " = {:v}", { v: value }),
    findAuthRecordByEmail: (collection, email) =>
      $app.findFirstRecordByFilter(collection, "email = {:email}", { email }),
    countRecords: (collection, ...exps) => $app.findAllRecords(collection, ...exps).length,
    save: (model) => {
      if (model instanceof Record) {
        model.__absorb(hostCall("saveRecord", model.__ref()));
      } else if (model && model.__data) {
        model.__data = Collection.__fromJSON(hostCall("saveCollection", model.__export())).__data;
      } else {
        throw new BadRequestError("$app.save expects a Record or a Collection");
      }
    },
    saveNoValidate: (model) => $app.save(model),
    delete: (model) => {
      if (model instanceof Record) {
        hostCall("deleteRecord", model.__ref());
      } else if (model && model.__data) {
        hostCall("deleteCollection", model.__data.id || model.__data.name);
      } else {
        throw new BadRequestError("$app.delete expects a Record or a Collection");
      }
    },
    runInTransaction: (fn) => {
      hostCall("txBegin");
      let ok = false;
      let error = null;
      try {
        fn($app);
        ok = true;
      } catch (err) {
        error = err;
      }
      hostCall("txEnd", ok, error ? String(error && error.message ? error.message : error) : "");
      if (error) throw error;
    },
    newMailClient: () => ({
      send: (message) => {
        const msg = Object.assign({}, message || {});
        if (msg.to && !Array.isArray(msg.to)) msg.to = [msg.to];
        hostCall("sendMail", msg);
      },
    }),
    logger: () => logger,
    store: () => ({
      get: (key) => hostCall("storeGet", String(key)),
      set: (key, value) => hostCall("storeSet", String(key), value === undefined ? null : value),
      has: (key) => hostCall("storeHas", String(key)),
      remove: (key) => hostCall("storeSet", String(key), null),
    }),
    cron: () => ({
      add: (id, expr, fn) => cronAdd(id, expr, fn),
      remove: (id) => cronRemove(id),
      mustAdd: (id, expr, fn) => cronAdd(id, expr, fn),
    }),
    dao: () => $app,
    isBootstrapped: () => true,
    isDev: () => false,
    dataDir: () => "",
    db: () => {
      throw new InternalServerError("raw database access ($app.db()) is not available in this runtime");
    },
    newBackupsFilesystem: () => {
      throw new InternalServerError("$app.newBackupsFilesystem is not available in this runtime");
    },
    newFilesystem: () => {
      throw new InternalServerError("$app.newFilesystem is not available in this runtime");
    },
  };

  // ---------------------------------------------------------------------
  // console / logger
  // ---------------------------------------------------------------------

  function formatArg(a) {
    if (typeof a === "string") return a;
    if (a instanceof Error) return a.stack || String(a);
    if (a === undefined) return "undefined";
    try {
      const s = JSON.stringify(a);
      return s === undefined ? String(a) : s;
    } catch (e) {
      return String(a);
    }
  }
  function kvData(pairs) {
    const data = {};
    for (let i = 0; i + 1 < pairs.length; i += 2) data[String(pairs[i])] = pairs[i + 1];
    if (pairs.length % 2 === 1) data["!BADKEY"] = pairs[pairs.length - 1];
    return data;
  }
  const logAt = (level) => (...args) => hostCall("log", level, args.map(formatArg).join(" "), {});
  const console = {
    log: logAt(0),
    info: logAt(0),
    warn: logAt(4),
    error: logAt(8),
    debug: logAt(-4),
  };
  const logger = {
    debug: (msg, ...kv) => hostCall("log", -4, String(msg), kvData(kv)),
    info: (msg, ...kv) => hostCall("log", 0, String(msg), kvData(kv)),
    warn: (msg, ...kv) => hostCall("log", 4, String(msg), kvData(kv)),
    error: (msg, ...kv) => hostCall("log", 8, String(msg), kvData(kv)),
  };

  // ---------------------------------------------------------------------
  // $http / $os / $security / $tokens / $mails / $filesystem / $apis
  // ---------------------------------------------------------------------

  const $http = {
    send: (config) => {
      const cfg = Object.assign({}, config || {});
      if (cfg.body !== undefined && cfg.body !== null && typeof cfg.body !== "string" && !Array.isArray(cfg.body)) {
        cfg.body = JSON.stringify(cfg.body);
        cfg.headers = Object.assign({ "content-type": "application/json" }, cfg.headers || {});
      }
      const res = hostCall("httpSend", cfg);
      res.cookies = {};
      res.body = res.raw;
      return res;
    },
  };

  const $os = {
    getenv: (name) => hostCall("os", "getenv", String(name)),
    readFile: (path) => hostCall("os", "readFile", String(path)),
    writeFile: (path, data) => hostCall("os", "writeFile", String(path), typeof data === "string" ? data : String(data)),
    exists: (path) => hostCall("os", "exists", String(path)),
    tempDir: () => hostCall("os", "tempDir"),
    getwd: () => hostCall("os", "getwd"),
    args: () => [],
    exit: (code) => console.warn("$os.exit(" + (code === undefined ? 0 : code) + ") ignored"),
    exec: () => {
      throw new ForbiddenError("$os.exec is disabled in this runtime");
    },
  };

  const $security = {
    randomString: (n) => hostCall("security", "randomString", n, ""),
    randomStringWithAlphabet: (n, alphabet) => hostCall("security", "randomString", n, String(alphabet)),
    pseudorandomString: (n) => hostCall("security", "randomString", n, ""),
    pseudorandomStringWithAlphabet: (n, alphabet) => hostCall("security", "randomString", n, String(alphabet)),
    randomStringByRegex: () => {
      throw new InternalServerError("$security.randomStringByRegex is not supported");
    },
    sha256: (s) => hostCall("security", "sha256", String(s)),
    sha512: (s) => hostCall("security", "sha512", String(s)),
    sha1: (s) => hostCall("security", "sha1", String(s)),
    md5: (s) => hostCall("security", "md5", String(s)),
    hs256: (data, secret) => hostCall("security", "hs256", String(data), String(secret)),
    hs512: (data, secret) => hostCall("security", "hs512", String(data), String(secret)),
    createJWT: (payload, key, secondsDuration, alg) =>
      hostCall("security", "createJWT", payload || {}, String(key), secondsDuration || 0, alg || "HS256"),
    parseUnverifiedJWT: (token) => hostCall("security", "parseUnverifiedJWT", String(token)),
    parseJWT: (token, key) => hostCall("security", "parseJWT", String(token), String(key)),
    encrypt: (data, key) => hostCall("security", "encrypt", String(data), String(key)),
    decrypt: (cipher, key) => hostCall("security", "decrypt", String(cipher), String(key)),
    equal: (a, b) => hostCall("security", "equal", String(a), String(b)),
  };

  const tokenFor = (kind) => (app, record) => {
    if (!(record instanceof Record)) throw new BadRequestError("$tokens expects a Record");
    return hostCall("recordToken", record.__ref(), kind);
  };
  const $tokens = {
    recordAuthToken: tokenFor("auth"),
    recordVerifyToken: tokenFor("verification"),
    recordResetPasswordToken: tokenFor("passwordReset"),
    recordChangeEmailToken: tokenFor("emailChange"),
    recordFileToken: tokenFor("file"),
  };

  const mailFor = (kind) => (app, record) => {
    if (!(record instanceof Record)) throw new BadRequestError("$mails expects a Record");
    hostCall("sendRecordMail", record.__ref(), kind);
  };
  const $mails = {
    sendRecordVerification: mailFor("verification"),
    sendRecordPasswordReset: mailFor("passwordReset"),
    sendRecordChangeEmail: mailFor("emailChange"),
    sendRecordOTP: mailFor("otp"),
  };

  function bytesOf(value) {
    if (typeof value === "string") return Array.from(new TextEncoder().encode(value));
    if (value instanceof ArrayBuffer) return Array.from(new Uint8Array(value));
    if (ArrayBuffer.isView(value)) return Array.from(value);
    if (Array.isArray(value)) return value.map(Number);
    return [];
  }
  const $filesystem = {
    fileFromPath: (path, name) => ({ __file: { path: String(path), name: name || String(path).split("/").pop() } }),
    fileFromBytes: (bytes, name) => ({ __file: { bytes: bytesOf(bytes), name: name || "file" } }),
    fileFromURL: (url, name) => ({ __file: { url: String(url), name: name || String(url).split("/").pop() } }),
    fileFromMultipart: () => {
      throw new InternalServerError("$filesystem.fileFromMultipart is not supported");
    },
  };

  const middleware = (id, func) => ({ id, func, __middleware: true });
  const $apis = {
    requireAuth: (...collections) =>
      middleware("pbRequireAuth", (e) => {
        if (!e.auth) throw new UnauthorizedError();
        if (collections.length && !collections.includes(e.auth.collectionName)) throw new ForbiddenError();
        return e.next();
      }),
    requireSuperuserAuth: () =>
      middleware("pbRequireSuperuserAuth", (e) => {
        if (!e.auth) throw new UnauthorizedError();
        if (e.auth.collectionName !== "_superusers") throw new ForbiddenError();
        return e.next();
      }),
    requireSuperuserOrOwnerAuth: (param) =>
      middleware("pbRequireSuperuserOrOwnerAuth", (e) => {
        if (!e.auth) throw new UnauthorizedError();
        const owner = e.request.pathValue(param || "id");
        if (e.auth.collectionName !== "_superusers" && e.auth.id !== owner) throw new ForbiddenError();
        return e.next();
      }),
    requireGuestOnly: () =>
      middleware("pbRequireGuestOnly", (e) => {
        if (e.auth) throw new BadRequestError("The request can be accessed only by guests.");
        return e.next();
      }),
    requireSameCollectionContextAuth: () => middleware("pbRequireSameCollectionContextAuth", (e) => e.next()),
    gzip: () => middleware("pbGzip", (e) => e.next()),
    bodyLimit: () => middleware("pbBodyLimit", (e) => e.next()),
    skipSuccessActivityLog: () => middleware("pbSkipSuccessActivityLog", (e) => e.next()),
    activityLogger: () => middleware("pbActivityLogger", (e) => e.next()),
    enrichRecord: (e, record) => record,
    enrichRecords: (e, records) => records,
    recordAuthResponse: () => {
      throw new InternalServerError("$apis.recordAuthResponse is not available in this runtime");
    },
    static: () => {
      throw new InternalServerError("$apis.static is not available in this runtime");
    },
    toApiError: (err) => (err && err.isApiError ? err : new BadRequestError(err && err.message ? err.message : String(err))),
  };

  // ---------------------------------------------------------------------
  // Registration tables
  // ---------------------------------------------------------------------

  const hooks = new Map(); // id -> {fn, kind, tags}
  const routes = new Map(); // id -> {fn, method, path, middlewares}
  const crons = new Map(); // id -> fn
  const counters = new Map();
  let globalMiddlewares = [];
  let moduleCache = new Map();

  function nextId(kind, key) {
    const file = __cb.currentFile || "<eval>";
    const ck = file + "|" + kind + "|" + key;
    const n = counters.get(ck) || 0;
    counters.set(ck, n + 1);
    return file + ":" + kind + ":" + key + ":" + n;
  }

  for (const name of __cb.hookNames) {
    global[name] = (handler, ...tags) => {
      if (typeof handler !== "function") throw new TypeError(name + " expects a function");
      const tagList = tags.flat().map(String);
      const id = nextId(name, tagList.join(","));
      hooks.set(id, { fn: handler, kind: name, tags: tagList });
      hostCall("registerHook", id, name, tagList, 0);
      return id;
    };
  }

  function routerAdd(method, path, handler, ...middlewares) {
    if (typeof handler !== "function") throw new TypeError("routerAdd expects a handler function");
    const m = String(method || "GET").toUpperCase();
    const id = nextId("route", m + " " + path);
    routes.set(id, { fn: handler, method: m, path: String(path), middlewares: middlewares.flat() });
    hostCall("registerRoute", id, m, String(path));
    return id;
  }
  function routerUse(...middlewares) {
    globalMiddlewares.push(...middlewares.flat());
  }
  function cronAdd(id, expr, fn) {
    if (typeof fn !== "function") throw new TypeError("cronAdd expects a function");
    crons.set(String(id), fn);
    hostCall("registerCron", String(id), String(expr));
  }
  function cronRemove(id) {
    crons.delete(String(id));
    hostCall("removeCron", String(id));
  }

  // ---------------------------------------------------------------------
  // require()
  // ---------------------------------------------------------------------

  function makeRequire(fromDir) {
    const require = (spec) => {
      const m = hostCall("loadModule", fromDir, String(spec));
      const cached = moduleCache.get(m.path);
      if (cached) return cached.exports;
      const module = { id: m.path, filename: m.path, exports: {}, loaded: false, children: [] };
      moduleCache.set(m.path, module);
      try {
        if (m.json) {
          module.exports = JSON.parse(m.source);
        } else {
          const fn = __cb.compile(m.path, m.source);
          fn.call(module.exports, module.exports, makeRequire(m.dir), module, m.path, m.dir);
        }
      } catch (err) {
        moduleCache.delete(m.path);
        throw err;
      }
      module.loaded = true;
      return module.exports;
    };
    require.resolve = (spec) => hostCall("loadModule", fromDir, String(spec)).path;
    require.cache = moduleCache;
    return require;
  }

  // ---------------------------------------------------------------------
  // Events
  // ---------------------------------------------------------------------

  const RESERVED = new Set([
    "record",
    "recordIsNew",
    "collection",
    "auth",
    "requestInfo",
    "request",
    "app",
    "next",
    "json",
    "string",
    "html",
    "redirect",
    "noContent",
    "blob",
    "response",
    "get",
    "set",
    "hasSuperuserAuth",
    "__data",
    "__nextCalled",
    "__response",
    "__store",
    "__pathParams",
  ]);

  function matchPath(pattern, path) {
    const params = {};
    const p = pattern.split("/").filter(Boolean);
    const s = path.split("/").filter(Boolean);
    for (let i = 0; i < p.length; i++) {
      const seg = p[i];
      if (seg === "{$}") return i === s.length ? params : null;
      const m = /^\{([^}]+?)(\.\.\.)?\}$/.exec(seg);
      if (m) {
        if (m[2]) {
          params[m[1]] = s.slice(i).join("/");
          return params;
        }
        if (i >= s.length) return null;
        params[m[1]] = decodeURIComponent(s[i]);
      } else if (seg !== s[i]) {
        return null;
      }
    }
    if (s.length > p.length && !pattern.endsWith("/")) return null;
    return params;
  }

  function makeRequest(data, e) {
    const headers = {};
    for (const k of Object.keys(data.headers || {})) headers[k.toLowerCase()] = data.headers[k];
    const query = data.query || {};
    const req = {
      method: data.method || "GET",
      headers,
      url: {
        path: data.path || "",
        query: () => ({ get: (k) => (query[k] === undefined ? "" : String(query[k])) }),
        string: () => data.path || "",
      },
      pathValue: (name) => (e.__pathParams && e.__pathParams[name] !== undefined ? String(e.__pathParams[name]) : ""),
      header: {
        // Nama header disimpan sisi Rust dengan `-` -> `_` (lihat
        // `extract.rs::header_map` -- supaya konsisten dipakai sebagai key
        // ekspresi filter). Konsumen JS wajar menulis nama header standar
        // ("X-Callback-Signature"), jadi lookup di sini menormalkan input
        // dengan cara yang sama sebelum mencocokkan, bukan mengharuskan
        // pemanggil tahu detail penyimpanan internal.
        get: (name) => headers[String(name).toLowerCase().replace(/-/g, "_")] || "",
      },
      body: data.body,
      rawBody: data.rawBody || "",
      remoteAddr: data.remoteIp || "",
    };
    return req;
  }

  function makeEvent(data) {
    data = data || {};
    const e = {};
    e.__data = data;
    e.__nextCalled = false;
    e.__response = null;
    e.__store = {};
    e.__pathParams = data.pathParams || null;
    for (const k of Object.keys(data)) {
      if (!RESERVED.has(k)) e[k] = data[k];
    }
    e.app = $app;
    if (data.record !== undefined && data.record !== null) e.record = recordFrom(data.record, { isNew: !!data.recordIsNew });
    if (data.collection !== undefined && data.collection !== null) e.collection = Collection.__fromJSON(data.collection);
    e.auth = data.auth ? recordFrom(data.auth) : null;
    const info = data.requestInfo || {};
    e.requestInfo = () => ({
      method: info.method || data.method || "",
      query: info.query || data.query || {},
      headers: info.headers || data.headers || {},
      body: info.body !== undefined ? info.body : data.body !== undefined ? data.body : {},
      auth: e.auth,
      context: info.context || "default",
    });
    e.request = makeRequest(data.request || { method: data.method, path: data.path, query: data.query, headers: data.headers, body: data.body, rawBody: data.rawBody, remoteIp: data.remoteIp }, e);
    e.next = () => {
      e.__nextCalled = true;
    };
    e.get = (key) => e.__store[key];
    e.set = (key, value) => {
      e.__store[key] = value;
    };
    e.hasSuperuserAuth = () => !!e.auth && e.auth.collectionName === "_superusers";
    e.realIP = () => (data.request && data.request.remoteIp) || data.remoteIp || "";
    const headers = {};
    e.response = {
      header: (name, value) => {
        if (value === undefined) return headers[String(name).toLowerCase()] || "";
        headers[String(name).toLowerCase()] = String(value);
        return "";
      },
      headers,
    };
    const respond = (status, body) => {
      e.__response = { status: status || 200, headers: Object.assign({}, headers), body };
    };
    e.json = (status, body) => respond(status, { kind: "json", value: body === undefined ? null : body });
    e.string = (status, text) => respond(status, { kind: "text", value: String(text) });
    e.html = (status, html) => respond(status, { kind: "html", value: String(html) });
    e.blob = (status, contentType, bytes) => {
      headers["content-type"] = String(contentType);
      respond(status, { kind: "bytes", value: bytesOf(bytes) });
    };
    e.redirect = (status, url) => respond(status || 302, { kind: "redirect", value: String(url) });
    e.noContent = (status) => respond(status || 204, { kind: "noContent" });
    e.error = (status, message, data) => new ApiError(status, message, data);
    e.badRequestError = (message, data) => new BadRequestError(message, data);
    e.notFoundError = (message, data) => new NotFoundError(message, data);
    e.forbiddenError = (message, data) => new ForbiddenError(message, data);
    e.unauthorizedError = (message, data) => new UnauthorizedError(message, data);
    e.tooManyRequestsError = (message, data) => new TooManyRequestsError(message, data);
    e.internalServerError = (message, data) => new InternalServerError(message, data);
    e.isPartial = () => data.method === "PATCH";
    return e;
  }

  function exportPlain(v) {
    if (v instanceof Record) return v.__export();
    if (v && v.__data && v.__export) return v.__export();
    return v;
  }

  function outcome(e) {
    const event = {};
    for (const k of Object.keys(e)) {
      if (RESERVED.has(k) || k.startsWith("__") || typeof e[k] === "function") continue;
      try {
        event[k] = JSON.parse(JSON.stringify(exportPlain(e[k])));
      } catch (err) {
        // non-serializable values are dropped
      }
    }
    return {
      nextCalled: e.__nextCalled,
      record: e.record ? e.record.__export() : null,
      dirtyFields: e.record ? [...e.record.__dirty] : [],
      collection: e.collection ? e.collection.__export() : null,
      response: e.__response,
      event,
    };
  }

  function runChain(e, chain) {
    let i = 0;
    const step = () => {
      const m = chain[i++];
      if (!m) return undefined;
      const fn = typeof m === "function" ? m : m && typeof m.func === "function" ? m.func : null;
      if (!fn) return step();
      return fn(e);
    };
    e.next = step;
    return step();
  }

  // ---------------------------------------------------------------------
  // Entry points called from Rust
  // ---------------------------------------------------------------------

  __cb.invokeHook = (id, data) => {
    const h = hooks.get(id);
    if (!h) throw new InternalServerError("unknown hook handler " + id);
    const e = makeEvent(data);
    h.fn(e);
    return outcome(e);
  };

  __cb.invokeRoute = (id, req) => {
    const r = routes.get(id);
    if (!r) throw new InternalServerError("unknown route handler " + id);
    const e = makeEvent(Object.assign({}, req, { request: req }));
    if (!e.__pathParams) {
      e.__pathParams = matchPath(r.path, req.path || "") || {};
    }
    runChain(e, [...globalMiddlewares, ...r.middlewares, r.fn]);
    if (!e.__response) e.noContent(204);
    return e.__response;
  };

  __cb.invokeCron = (id) => {
    const fn = crons.get(id);
    if (!fn) throw new InternalServerError("unknown cron job " + id);
    fn();
  };

  __cb.runMigration = (direction) => {
    const m = __cb.pendingMigration;
    if (!m) throw new BadRequestError("migration file did not call migrate()");
    const fn = direction === "up" ? m.up : m.down;
    if (typeof fn !== "function") return;
    $app.runInTransaction((app) => fn(app));
  };

  __cb.reset = () => {
    hooks.clear();
    routes.clear();
    crons.clear();
    counters.clear();
    globalMiddlewares = [];
    moduleCache = new Map();
    global.require = makeRequire(__cb.hooksDir);
  };

  function migrate(up, down) {
    __cb.pendingMigration = { up, down };
  }

  // ---------------------------------------------------------------------
  // Globals
  // ---------------------------------------------------------------------

  Object.assign(global, {
    ApiError,
    BadRequestError,
    UnauthorizedError,
    ForbiddenError,
    NotFoundError,
    TooManyRequestsError,
    InternalServerError,
    Record,
    Collection,
    DateTime,
    $app,
    $http,
    $os,
    $security,
    $tokens,
    $mails,
    $filesystem,
    $apis,
    $dbx,
    console,
    routerAdd,
    routerUse,
    cronAdd,
    cronRemove,
    migrate,
    sleep: (ms) => hostCall("sleep", Number(ms) || 0),
    toString: (v) => (v === undefined || v === null ? "" : typeof v === "string" ? v : Array.isArray(v) ? String.fromCharCode(...v) : String(v)),
    readerToString: (v) => (v === undefined || v === null ? "" : String(v)),
    toBytes: (v) => bytesOf(v),
    __hooks: __cb.hooksDir,
    require: makeRequire(__cb.hooksDir),
    nullString: (v) => (v === undefined ? null : v),
    nullInt: (v) => (v === undefined ? null : v),
    nullFloat: (v) => (v === undefined ? null : v),
    nullBool: (v) => (v === undefined ? null : v),
  });
})(globalThis);
