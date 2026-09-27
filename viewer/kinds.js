"use strict";

// Experimental: a garment kind read from the album title, so the corpus can
// be browsed by kind across sellers whose own Yupoo categories differ or
// are empty. A keyword table, nothing cleverer: the first kind in table
// order with a token in the title wins, and a title with none is "unsorted"
// rather than guessed. Latin tokens match on word boundaries, CJK tokens
// anywhere. ponytail: precision is unmeasured on real catalogues; the
// filter is labelled experimental and an album's own category filter is
// untouched.

/** @typedef {{id: string, label: string, latin: string[], cjk: string[]}} Kind */

/** @type {Kind[]} */
const KINDS = [
  { id: "sets", label: "Sets", latin: ["set", "sets", "tracksuit", "tracksuits", "suit", "suits", "outfit"],
    cjk: ["套装", "两件套", "三件套"] },
  { id: "shoes", label: "Shoes", latin: ["shoe", "shoes", "sneaker", "sneakers", "boot", "boots", "slide",
    "slides", "sandal", "sandals", "loafer", "loafers", "runner", "runners", "trainer", "trainers", "mule",
    "mules", "jordan", "dunk", "dunks", "yeezy", "af1", "sb", "gel", "kayano", "samba", "gazelle", "clog", "clogs"],
    cjk: ["鞋", "靴", "拖鞋", "凉鞋"] },
  { id: "jackets", label: "Jackets and coats", latin: ["jacket", "jackets", "coat", "coats", "parka", "puffer",
    "puffers", "windbreaker", "blazer", "vest", "gilet", "bomber", "trench", "anorak", "down", "fleece", "overshirt"],
    cjk: ["外套", "夹克", "羽绒", "大衣", "风衣", "马甲", "棉服", "冲锋衣"] },
  { id: "hoodies", label: "Hoodies and sweaters", latin: ["hoodie", "hoodies", "hoody", "sweatshirt", "sweatshirts",
    "crewneck", "sweater", "sweaters", "knit", "knitwear", "cardigan", "pullover", "jumper", "zip-up", "zipup"],
    cjk: ["卫衣", "毛衣", "针织", "开衫", "连帽"] },
  { id: "tees", label: "Tees and shirts", latin: ["tee", "tees", "t-shirt", "tshirt", "t-shirts", "shirt", "shirts",
    "polo", "polos", "tank", "jersey", "jerseys", "longsleeve", "long-sleeve"],
    cjk: ["短袖", "T恤", "衬衫", "长袖", "球衣", "背心"] },
  { id: "pants", label: "Pants", latin: ["pants", "jeans", "denim", "trousers", "joggers", "sweatpants", "cargo",
    "cargos", "chinos", "leggings", "tights"],
    cjk: ["裤", "牛仔"] },
  { id: "shorts", label: "Shorts", latin: ["shorts", "swim", "trunks"], cjk: ["短裤", "泳裤"] },
  { id: "bags", label: "Bags and wallets", latin: ["bag", "bags", "backpack", "backpacks", "tote", "totes", "wallet",
    "wallets", "cardholder", "card holder", "purse", "pouch", "crossbody", "handbag", "duffle", "duffel", "clutch",
    "briefcase", "luggage", "suitcase"],
    cjk: ["包", "钱包", "卡包", "背包", "箱"] },
  { id: "watches", label: "Watches", latin: ["watch", "watches", "chronograph"], cjk: ["手表", "腕表"] },
  { id: "jewellery", label: "Jewellery", latin: ["ring", "rings", "necklace", "necklaces", "bracelet", "bracelets",
    "earring", "earrings", "chain", "chains", "pendant", "brooch", "cufflinks", "jewelry", "jewellery"],
    cjk: ["项链", "手链", "戒指", "耳环", "耳钉", "首饰", "吊坠"] },
  { id: "accessories", label: "Accessories", latin: ["hat", "hats", "cap", "caps", "beanie", "beanies", "belt", "belts",
    "scarf", "scarves", "socks", "glasses", "sunglasses", "eyewear", "gloves", "tie", "ties", "keychain", "bandana",
    "balaclava", "umbrella", "perfume", "airpods", "headphones", "case", "cover"],
    cjk: ["帽", "皮带", "腰带", "围巾", "袜", "眼镜", "墨镜", "手套", "领带", "香水", "耳机"] },
];
const UNSORTED = "unsorted";
const KIND_IDS = [...KINDS.map(kind => kind.id), UNSORTED];

// A part or container named after the thing it serves ("watch strap",
// "shoelaces", "jewellery box") is an accessory, whatever else the title
// says, so these are checked before the table.
/** @type {Kind} */
const PARTS = { id: "accessories", label: "Accessories", latin: ["strap", "straps", "laces", "shoelaces", "insole",
  "insoles", "box", "boxes", "organizer", "organiser", "cleaner", "protector"],
  cjk: ["表带", "鞋带", "鞋垫", "盒", "收纳"] };

/** @param {Kind} kind */
const latinPattern = kind => new RegExp(
  "(?:^|[^a-z0-9])(?:" + kind.latin.map(token => token.replace(/[.*+?^${}()|[\]\\-]/g, "\\$&")).join("|") +
  ")(?![a-z0-9])", "i");
const LATIN_PATTERNS = KINDS.map(latinPattern);
const PARTS_PATTERN = latinPattern(PARTS);

/** @param {Kind} kind @param {RegExp} pattern @param {string} text */
const mentions = (kind, pattern, text) => pattern.test(text) || kind.cjk.some(token => text.includes(token));

/**
 * The kind id for a title, "unsorted" when no token matches. Links in the
 * title are ignored, so a shop path never names a kind.
 * @param {string | null | undefined} title
 */
function kindOf(title) {
  const text = (title || "").normalize("NFKC").toLowerCase().replace(/(?:https?:\/\/)?[a-z0-9.-]+\.[a-z]{2,}\/\S*/g, " ");
  if (!text.trim()) return UNSORTED;
  if (mentions(PARTS, PARTS_PATTERN, text)) return PARTS.id;
  for (let i = 0; i < KINDS.length; i++) {
    if (mentions(KINDS[i], LATIN_PATTERNS[i], text)) return KINDS[i].id;
  }
  return UNSORTED;
}

if (typeof module !== "undefined") {
  module.exports = { KINDS, KIND_IDS, UNSORTED, kindOf };
}
