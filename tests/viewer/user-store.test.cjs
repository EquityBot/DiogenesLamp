const test = require("node:test");
const assert = require("node:assert/strict");
const { createUserStore, importUserBackup, parseUserBackup } = require("../../viewer/user-store.js");
function host() {
  const data = new Map();
  return { localStorage: { get length() { return data.size; },
    key: i => [...data.keys()][i] ?? null, getItem: k => data.get(k) ?? null,
    setItem: (k,v) => data.set(k,v) } };
}
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
