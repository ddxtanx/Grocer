-- Grocer core schema (phase 1)

CREATE TABLE users (
  id BIGSERIAL PRIMARY KEY,
  username TEXT UNIQUE NOT NULL,
  pass_hash TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE sessions (
  id BIGSERIAL PRIMARY KEY,
  user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  token_hash TEXT UNIQUE NOT NULL,
  expires_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE chains (
  id BIGSERIAL PRIMARY KEY,
  name TEXT UNIQUE NOT NULL,
  parser_key TEXT UNIQUE NOT NULL
);

CREATE TABLE stores (
  id BIGSERIAL PRIMARY KEY,
  chain_id BIGINT NOT NULL REFERENCES chains(id),
  name TEXT NOT NULL,
  address TEXT,
  city TEXT,
  store_ref TEXT,
  lat DOUBLE PRECISION,
  lon DOUBLE PRECISION,
  UNIQUE (chain_id, store_ref)
);

CREATE TABLE products (
  id BIGSERIAL PRIMARY KEY,
  name TEXT NOT NULL,
  brand TEXT,
  category TEXT,
  upc TEXT,
  pack_size NUMERIC,          -- numeric size in size_unit
  size_unit TEXT              -- 'g' | 'ml' | 'ea'
);

CREATE TABLE trips (
  id BIGSERIAL PRIMARY KEY,
  user_id BIGINT NOT NULL REFERENCES users(id),
  store_id BIGINT NOT NULL REFERENCES stores(id),
  occurred_at TEXT NOT NULL,  -- ISO local time as printed on the receipt
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE receipts (
  id BIGSERIAL PRIMARY KEY,
  user_id BIGINT NOT NULL REFERENCES users(id),
  trip_id BIGINT REFERENCES trips(id),
  store_id BIGINT NOT NULL REFERENCES stores(id),
  chain_id BIGINT NOT NULL REFERENCES chains(id),
  txn_ref TEXT,
  occurred_at TEXT,
  subtotal_cents BIGINT,
  tax_cents BIGINT,
  total_cents BIGINT,
  discount_cents BIGINT,
  item_count INT,
  total_source TEXT,
  raw_text TEXT,
  parser_version TEXT,
  issues JSONB NOT NULL DEFAULT '[]',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON receipts (user_id);

CREATE TABLE receipt_items (
  id BIGSERIAL PRIMARY KEY,
  receipt_id BIGINT NOT NULL REFERENCES receipts(id) ON DELETE CASCADE,
  seq INT NOT NULL,
  raw_name TEXT NOT NULL,
  store_item_code TEXT,
  discount_ref TEXT,
  qty_count INT,
  qty_grams NUMERIC,
  unit_basis TEXT,
  unit_price_cents BIGINT,
  line_total_cents BIGINT NOT NULL,
  savings_cents BIGINT,
  flags JSONB NOT NULL DEFAULT '[]',
  department TEXT,
  confidence REAL,
  product_id BIGINT REFERENCES products(id)
);
CREATE INDEX ON receipt_items (receipt_id);
CREATE INDEX ON receipt_items (store_item_code);

-- Normalized price facts; source: receipt (observed truth) | scraper | manual
CREATE TABLE price_observations (
  id BIGSERIAL PRIMARY KEY,
  store_id BIGINT NOT NULL REFERENCES stores(id),
  chain_id BIGINT NOT NULL REFERENCES chains(id),
  receipt_item_id BIGINT REFERENCES receipt_items(id) ON DELETE CASCADE,
  product_id BIGINT REFERENCES products(id),
  store_item_code TEXT,
  raw_name TEXT,
  qty_count INT,
  qty_grams NUMERIC,
  unit_price_cents BIGINT NOT NULL,
  observed_at TEXT NOT NULL,
  source TEXT NOT NULL DEFAULT 'receipt'
    CHECK (source IN ('receipt','scraper','manual'))
);
CREATE INDEX ON price_observations (product_id);
CREATE INDEX ON price_observations (chain_id, store_item_code);

-- community-confirmed: chain item code -> canonical product (the free within-chain key)
CREATE TABLE store_item_map (
  id BIGSERIAL PRIMARY KEY,
  chain_id BIGINT NOT NULL REFERENCES chains(id),
  store_item_code TEXT NOT NULL,
  product_id BIGINT NOT NULL REFERENCES products(id),
  confirmations INT NOT NULL DEFAULT 1,
  UNIQUE (chain_id, store_item_code, product_id)
);

-- barcode-assisted anchoring: UPC scanned at the shelf -> canonical product
CREATE TABLE barcode_links (
  id BIGSERIAL PRIMARY KEY,
  upc TEXT UNIQUE NOT NULL,
  product_id BIGINT NOT NULL REFERENCES products(id),
  source TEXT NOT NULL DEFAULT 'scan',
  confirmations INT NOT NULL DEFAULT 1
);

CREATE TABLE shelf_life_defaults (
  id BIGSERIAL PRIMARY KEY,
  category TEXT UNIQUE NOT NULL,
  days INT NOT NULL
);
INSERT INTO shelf_life_defaults (category, days) VALUES
  ('leafy-greens', 6), ('dairy', 14), ('yogurt', 21), ('cheese', 35),
  ('produce', 10), ('frozen', 270), ('meat', 3), ('bakery', 5),
  ('pantry', 180), ('beverage', 120), ('snack', 90), ('condiment', 200);

CREATE TABLE shelf_life_overrides (
  product_id BIGINT PRIMARY KEY REFERENCES products(id),
  days INT NOT NULL
);

CREATE TABLE fridge_items (
  id BIGSERIAL PRIMARY KEY,
  user_id BIGINT NOT NULL REFERENCES users(id),
  product_id BIGINT NOT NULL REFERENCES products(id),
  qty INT NOT NULL DEFAULT 1,
  acquired_at DATE NOT NULL DEFAULT CURRENT_DATE,
  expires_at DATE,
  consumed_at TIMESTAMPTZ,
  notes TEXT
);
CREATE INDEX ON fridge_items (user_id, expires_at);

CREATE TABLE shopping_lists (
  id BIGSERIAL PRIMARY KEY,
  user_id BIGINT NOT NULL REFERENCES users(id),
  name TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE shopping_list_items (
  id BIGSERIAL PRIMARY KEY,
  list_id BIGINT NOT NULL REFERENCES shopping_lists(id) ON DELETE CASCADE,
  product_id BIGINT NOT NULL REFERENCES products(id),
  qty INT NOT NULL DEFAULT 1,
  note TEXT,
  done BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE trip_plans (
  id BIGSERIAL PRIMARY KEY,
  list_id BIGINT NOT NULL REFERENCES shopping_lists(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  total_cents BIGINT
);
CREATE TABLE plan_stops (
  id BIGSERIAL PRIMARY KEY,
  plan_id BIGINT NOT NULL REFERENCES trip_plans(id) ON DELETE CASCADE,
  store_id BIGINT NOT NULL REFERENCES stores(id),
  position INT NOT NULL
);
CREATE TABLE plan_items (
  id BIGSERIAL PRIMARY KEY,
  plan_id BIGINT NOT NULL REFERENCES trip_plans(id) ON DELETE CASCADE,
  product_id BIGINT NOT NULL REFERENCES products(id),
  store_id BIGINT NOT NULL REFERENCES stores(id),
  qty INT NOT NULL,
  unit_price_cents BIGINT NOT NULL
);

-- Shelf-tag advertised prices (from photographed tags, OCR'd on-device; photo stays local)
CREATE TABLE price_checks (
  id BIGSERIAL PRIMARY KEY,
  user_id BIGINT NOT NULL REFERENCES users(id),
  store_id BIGINT NOT NULL REFERENCES stores(id),
  trip_id BIGINT REFERENCES trips(id),
  product_id BIGINT REFERENCES products(id),
  upc TEXT,
  advertised_cents BIGINT NOT NULL,
  unit_basis TEXT DEFAULT 'EA',
  confidence REAL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON price_checks (store_id, product_id);

-- Seed the chains + stores from the sample receipts
INSERT INTO chains (name, parser_key) VALUES
  ('Fresh Thyme', 'fresh_thyme'),
  ('Trader Joe''s', 'trader_joes'),
  ('Costco', 'costco');

INSERT INTO stores (chain_id, name, address, city, store_ref, lat, lon) VALUES
  (1, 'Fresh Thyme St. Louis Park', '4840 Excelsior Blvd.', 'St. Louis Park', '00905', 44.9562, -93.3160),
  (2, 'Trader Joe''s Downtown Mpls', '721 Washington Ave S Suite 101', 'Minneapolis', '0725', 44.9765, -93.2647),
  (3, 'Costco Minneapolis BD', '3311 Broadway St NE', 'Minneapolis', '652', 45.0023, -93.2450);
