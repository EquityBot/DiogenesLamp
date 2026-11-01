const test = require("node:test");
const assert = require("node:assert/strict");
const { createUserStore, importUserBackup, parseUserBackup, inheritDuplicateWork } = require("../../viewer/user-store.js");
function host() {
  const data = new Map();
  return { localStorage: { get length() { return data.size; },
    key: i => [...data.keys()][i] ?? null, getItem: k => data.get(k) ?? null,
    setItem: (k,v) => data.set(k,v) } };
}

test("newest duplicate inherits missing fields, preserves removals and keeps originals", () => {
  const store = createUserStore(host());
  store.set("favorite", "100", true);
  store.set("note", "100", "Earlier note");
  store.set("note", "200", "Later note");
  store.set("tags", "100", ["Compare"]);
  store.set("collection", "100", "Shortlist");
  store.set("favorite", "300", false);
  store.set("tags", "300", []);
  assert.equal(inheritDuplicateWork(store, {100: "300", 200: "300"}), 2);
  assert.equal(store.get("favorite", "300"), false);
  assert.deepEqual(store.get("tags", "300"), []);
  assert.equal(store.get("note", "300"), "Later note");
  assert.equal(store.get("collection", "300"), "Shortlist");
  assert.equal(store.get("note", "100"), "Earlier note");
  assert.equal(inheritDuplicateWork(store, {100: "300", 200: "300"}), 0);
  store.set("note", "300", "");
  assert.equal(inheritDuplicateWork(store, {100: "300", 200: "300"}), 0);
  assert.equal(store.get("note", "300"), "");
});

test("duplicate inheritance survives a failed write and resumes without erasing data", () => {
  const shared = host(), errors = [], store = createUserStore(shared, x => errors.push(x));
  store.set("note", "100", "Keep this note");
  const write = shared.localStorage.setItem;
  shared.localStorage.setItem = () => { throw Error("quota"); };
  assert.equal(inheritDuplicateWork(store, {100: "200"}), 0);
  assert.equal(store.get("note", "100"), "Keep this note");
  assert.equal(store.get("note", "200"), undefined);
  assert.ok(errors.length);
  shared.localStorage.setItem = write;
  assert.equal(inheritDuplicateWork(store, {100: "200"}), 1);
});

test("automatic inheritance preserves concurrent explicit values in both stores and backups", () => {
  for (const [kind, original, explicit] of [
    ["note", "Older note", "Explicit note"], ["note", "Older note", ""],
    ["favorite", true, false], ["tags", ["Older"], []], ["collection", "Older", ""]
  ]) {
    const shared = host(), a = createUserStore(shared), b = createUserStore(shared);
    a.set(kind, "100", original);
    const key = "lamp.user.v1." + kind + ".200", read = shared.localStorage.getItem;
    let injected = false;
    shared.localStorage.getItem = name => {
      const raw = read(name);
      if (name === key && raw === null && !injected) {
        injected = true;
        assert.equal(b.set(kind, "200", explicit), true);
      }
      return raw;
    };
    inheritDuplicateWork(a, {100: "200"});
    assert.equal(injected, true);
    assert.equal(read(key), JSON.stringify(explicit));
    for (const store of [a, b, createUserStore(shared)]) {
      assert.deepEqual(store.get(kind, "200"), explicit);
      assert.deepEqual(store.load().filter(e => e.kind === kind && e.id === "200"),
        [{kind, id: "200", value: explicit}]);
      assert.deepEqual(store.get(kind, "100"), original);
    }
  }
});

test("inherited entries persist, export once and preserve historical and later explicit values", () => {
  const shared = host(), store = createUserStore(shared);
  store.set("note", "100", "Original");
  store.set("note", "150", "Higher older ID");
  assert.equal(inheritDuplicateWork(store, {100: "200", 150: "200"}), 1);
  assert.equal(shared.localStorage.getItem("lamp.user.v1.note.200"), null);
  const reloaded = createUserStore(shared);
  assert.equal(reloaded.get("note", "200"), "Higher older ID");
  assert.equal(reloaded.has("note", "200"), true);
  assert.equal(inheritDuplicateWork(reloaded, {}), 0);
  assert.equal(inheritDuplicateWork(reloaded, {100: "200", 150: "200"}), 0);
  const backup = {format: "diogeneslamp", version: 1, entries: reloaded.load()};
  const restored = createUserStore(host());
  assert.equal(importUserBackup(restored, JSON.parse(JSON.stringify(backup))), 3);
  assert.deepEqual(restored.load(), backup.entries);
  assert.equal(importUserBackup(reloaded, backup), 0);
  assert.equal(reloaded.set("note", "200", ""), true);
  assert.equal(store.get("note", "200"), "");
  assert.equal(createUserStore(shared).get("note", "200"), "");
  assert.equal(reloaded.load().filter(e => e.id === "200").length, 1);
  shared.localStorage.setItem("lamp.user.v1.note.200", '"Historical copy"');
  assert.equal(createUserStore(shared).get("note", "200"), "Historical copy");
  assert.equal(inheritDuplicateWork(store, {100: "200", 150: "200"}), 0);
});

test("corrupt or unreadable explicit entries block inherited fallback and copying", () => {
  const shared = host(), errors = [], store = createUserStore(shared, e => errors.push(e));
  store.set("note", "100", "Original");
  assert.equal(inheritDuplicateWork(store, {100: "200"}), 1);
  const key = "lamp.user.v1.note.200", corrupt = "{broken";
  shared.localStorage.setItem(key, corrupt);
  assert.equal(store.has("note", "200"), true);
  assert.equal(store.get("note", "200"), undefined);
  assert.equal(store.load().some(e => e.id === "200"), false);
  assert.equal(inheritDuplicateWork(store, {100: "200"}), 0);
  assert.equal(shared.localStorage.getItem(key), corrupt);
  const read = shared.localStorage.getItem;
  shared.localStorage.getItem = name => {
    if (name === key) throw Error("read unavailable");
    return read(name);
  };
  assert.throws(() => store.has("note", "200"), /read unavailable/);
  assert.equal(store.get("note", "200"), undefined);
  assert.equal(inheritDuplicateWork(store, {100: "200"}), 0);
  assert.equal(read(key), corrupt);
  assert.equal(read("lamp.user.v1.note.100"), '"Original"');
  assert.ok(errors.length);
});
test("two tabs update different albums without overwriting each other", () => {
  const shared = host(), a = createUserStore(shared), b = createUserStore(shared);
  a.set("favorite", "101", true);
  b.set("favorite", "102", true);
  assert.deepEqual(a.favorites(), new Set(["101","102"]));
  a.set("favorite", "101", false);
  b.set("note", "102", "Compare the stitching");
  assert.deepEqual(b.favorites(), new Set(["102"]));
  assert.equal(a.get("note", "102"), "Compare the stitching");
});
test("one corrupt entry preserves other saved work and reports its failure", () => {
  const shared = host(), errors = [];
  shared.localStorage.setItem("lamp.user.v1.favorite.101", "true");
  shared.localStorage.setItem("lamp.user.v1.note.102", "{bad");
  const store = createUserStore(shared, error => errors.push(error));
  assert.deepEqual(store.favorites(), new Set(["101"]));
  assert.ok(errors.length > 0);
});
test("legacy migration preserves removals and does not resurrect them", () => {
  const shared = host();
  shared.localStorage.setItem("candle.favorites", '["101","102"]');
  const a = createUserStore(shared);
  assert.deepEqual(a.favorites(), new Set(["101","102"]));
  a.set("favorite", "101", false);
  assert.deepEqual(createUserStore(shared).favorites(), new Set(["102"]));
});
test("backup round-trip retains unavailable IDs, notes, collections and review progress", () => {
  const a = createUserStore(host());
  a.set("favorite","999999",true);
  a.set("note","999999","Unavailable seller");
  a.set("collection","999999","Compare");
  a.set("review","sellerx",123456);
  const backup = { format:"diogeneslamp", version:1, entries:a.load(), albums:[] };
  const b = createUserStore(host());
  assert.equal(importUserBackup(b, backup),4);
  assert.deepEqual(b.load(),a.load());
  b.set("note","999999","Newer note");
  assert.equal(importUserBackup(b, backup),0);
  assert.equal(b.get("note","999999"),"Newer note");
});
test("malformed backups cause no writes and unsupported versions fail", () => {
  const a = createUserStore(host());
  assert.throws(() => importUserBackup(a, {format:"diogeneslamp",version:1,
    entries:[{kind:"favorite",id:"101",value:true},{kind:"note",id:"102",value:4}]}));
  assert.equal(a.load().length,0);
  assert.throws(() => parseUserBackup({format:"diogeneslamp",version:2,entries:[]}));
});
test("storage failures are visible and do not pretend to save", () => {
  const errors=[];
  const a=createUserStore({get localStorage(){throw Error("blocked");}},m=>errors.push(m));
  assert.equal(a.set("favorite","101",true),false);
  assert.equal(a.favorites().size,0);
  assert.ok(errors.length);
});
test("older favourites exports can be imported", () => {
  const a=createUserStore(host());
  assert.equal(importUserBackup(a,{albums:[{id:"101",title:"A"}]}),1);
  assert.deepEqual(a.favorites(),new Set(["101"]));
});

test("backup import preserves an unreadable existing entry", () => {
  const shared = host(), raw = '{"interrupted';
  shared.localStorage.setItem("lamp.user.v1.note.101", raw);
  const store = createUserStore(shared);
  assert.equal(importUserBackup(store, {format:"diogeneslamp",version:1,
    entries:[{kind:"note",id:"101",value:"Old backup"}]}), 0);
  assert.equal(shared.localStorage.getItem("lamp.user.v1.note.101"), raw);
});

test("read failure stops import without overwriting an uncached existing field", () => {
  const shared = host(), store = createUserStore(shared);
  const key = "lamp.user.v1.note.102";
  shared.localStorage.setItem(key, '"Newer note from another tab"');
  const read = shared.localStorage.getItem;
  shared.localStorage.getItem = name => {
    if (name === key) throw Error("read unavailable");
    return read(name);
  };
  assert.throws(() => importUserBackup(store, {format:"diogeneslamp",version:1,entries:[
    {kind:"favorite",id:"101",value:true}, {kind:"note",id:"102",value:"Old backup"}
  ]}), /1 entries were imported/);
  assert.equal(read(key), '"Newer note from another tab"');
  assert.equal(store.get("favorite", "101"), true);
});

test("backup accepts 100000 distinct entries and rejects overflow before touching storage", () => {
  const entries = Array.from({length: 100000}, (_, i) =>
    ({kind: "favorite", id: String(i + 1), value: true}));
  const backup = {format: "diogeneslamp", version: 1, entries};
  assert.equal(parseUserBackup(backup), entries);
  const untouched = {
    has() { assert.fail("an oversized backup must not read storage"); },
    set() { assert.fail("an oversized backup must not write storage"); }
  };
  assert.throws(() => importUserBackup(untouched, {...backup,
    entries: [...entries, {kind: "favorite", id: "100001", value: true}]}), /too many/);
});

test("a malformed or duplicate final entry rejects a large backup before storage access", () => {
  const entries = Array.from({length: 99999}, (_, i) =>
    ({kind: "favorite", id: String(i + 1), value: true}));
  const untouched = {
    has() { assert.fail("validation must finish before reading storage"); },
    set() { assert.fail("validation must finish before writing storage"); }
  };
  for (const [last, error] of [
    [{kind: "note", id: "100000", value: 4}, /invalid saved entry/],
    [{kind: "favorite", id: "1", value: true}, /repeats a saved entry/]
  ]) {
    assert.throws(() => importUserBackup(untouched,
      {format: "diogeneslamp", version: 1, entries: [...entries, last]}), error);
  }
});

test("a quota interruption reports its saved prefix and retries only the missing entries", () => {
  const shared = host(), errors = [], store = createUserStore(shared, e => errors.push(e));
  store.set("note", "101", "Newer note");
  store.set("favorite", "102", false);
  const write = shared.localStorage.setItem;
  let writes = 0;
  shared.localStorage.setItem = (key, value) => {
    if (writes === 2) throw Error("quota");
    write(key, value);
    writes++;
  };
  const backup = {format: "diogeneslamp", version: 1, entries: [
    {kind: "note", id: "101", value: "Older note"},
    {kind: "favorite", id: "102", value: true},
    ...[103, 104, 105, 106].map(id => ({kind: "note", id: String(id), value: "Saved note"}))
  ]};
  assert.throws(() => importUserBackup(store, backup), /2 entries were imported/);
  assert.equal(writes, 2);
  assert.ok(errors.some(error => error.includes("quota")));
  shared.localStorage.setItem = write;
  const resumed = createUserStore(shared);
  assert.equal(resumed.get("note", "101"), "Newer note");
  assert.equal(resumed.get("favorite", "102"), false);
  assert.equal(resumed.get("note", "103"), "Saved note");
  assert.equal(resumed.get("note", "104"), "Saved note");
  assert.equal(resumed.has("note", "105"), false);
  assert.equal(resumed.has("note", "106"), false);
  assert.equal(importUserBackup(resumed, backup), 2);
  assert.equal(importUserBackup(createUserStore(shared), backup), 0);
  assert.equal(resumed.get("note", "101"), "Newer note");
  assert.equal(resumed.get("favorite", "102"), false);
  assert.equal(resumed.load().length, 6);
});
