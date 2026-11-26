// SPDX-License-Identifier: GPL-3.0-or-later

//! Print a deterministic JSON snapshot of the live service registry.
//!
//! This only constructs the registry. It does not load a PE image, execute
//! guest instructions or call a service handler.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::sync::Arc;
use xenora_loader::import_table::ImportKind;
use xenora_loader::imports::{ImportRegistry, ServiceDescriptor};
use xenora_loader::runtime_table::SERVICES;
use xenora_loader::services::{MockClock, ServicesState, register_standard_services};

struct KeyRow {
    library: String,
    symbol: String,
    kind: ImportKind,
    canonical_library: String,
    canonical_symbol: String,
    canonical_kind: ImportKind,
    canonical: bool,
    api_set: bool,
}

fn push_json_string(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if character <= '\u{1f}' => {
                write!(out, "\\u{:04x}", character as u32).expect("write to string");
            }
            character => out.push(character),
        }
    }
    out.push('"');
}

fn push_json_bool(out: &mut String, value: bool) {
    out.push_str(if value { "true" } else { "false" });
}

fn api_set(module: &str) -> bool {
    module.starts_with("api-ms-") || module.starts_with("ext-ms-")
}

fn service_row(out: &mut String, service: &ServiceDescriptor, typed: &BTreeSet<(String, String)>) {
    let source = if service.kind == ImportKind::Name
        && typed.contains(&(service.module.clone(), service.symbol.clone()))
    {
        "typed"
    } else {
        "manual"
    };
    out.push_str("    {\n      \"library\": ");
    push_json_string(out, &service.module);
    out.push_str(",\n      \"symbol\": ");
    push_json_string(out, &service.symbol);
    out.push_str(",\n      \"kind\": ");
    push_json_string(out, service.kind.as_str());
    out.push_str(",\n      \"status\": ");
    push_json_string(out, service.status.as_str());
    out.push_str(",\n      \"status_explicit\": ");
    push_json_bool(out, service.status_explicit);
    out.push_str(",\n      \"source\": ");
    push_json_string(out, source);
    out.push_str(",\n      \"data_import\": ");
    push_json_bool(out, service.data_import);
    out.push_str("\n    }");
}

fn key_row(out: &mut String, row: &KeyRow) {
    out.push_str("    {\n      \"library\": ");
    push_json_string(out, &row.library);
    out.push_str(",\n      \"symbol\": ");
    push_json_string(out, &row.symbol);
    out.push_str(",\n      \"kind\": ");
    push_json_string(out, row.kind.as_str());
    out.push_str(",\n      \"canonical_library\": ");
    push_json_string(out, &row.canonical_library);
    out.push_str(",\n      \"canonical_symbol\": ");
    push_json_string(out, &row.canonical_symbol);
    out.push_str(",\n      \"canonical_kind\": ");
    push_json_string(out, row.canonical_kind.as_str());
    out.push_str(",\n      \"canonical\": ");
    push_json_bool(out, row.canonical);
    out.push_str(",\n      \"api_set\": ");
    push_json_bool(out, row.api_set);
    out.push_str("\n    }");
}

fn typed_row(out: &mut String, service: &xenora_loader::runtime::ServiceMetadata) {
    out.push_str("    {\n      \"library\": ");
    push_json_string(out, service.library);
    out.push_str(",\n      \"symbol\": ");
    push_json_string(out, service.symbol);
    out.push_str(",\n      \"kind\": \"name\"");
    out.push_str(",\n      \"status\": ");
    push_json_string(out, service.status.as_str());
    out.push_str(",\n      \"aliases\": [");
    for (index, alias) in service.aliases.iter().enumerate() {
        if index != 0 {
            out.push_str(", ");
        }
        push_json_string(out, alias);
    }
    out.push_str("]\n    }");
}

fn snapshot(registry: &ImportRegistry) -> String {
    let typed_keys: BTreeSet<(String, String)> = SERVICES
        .iter()
        .map(|service| {
            (
                service.library.to_ascii_lowercase(),
                service.symbol.to_owned(),
            )
        })
        .collect();
    let mut services: Vec<&ServiceDescriptor> = registry.iter_service_descriptors().collect();
    services.sort_by(|left, right| {
        left.module
            .cmp(&right.module)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.symbol.cmp(&right.symbol))
    });

    let mut keys: Vec<KeyRow> = registry
        .iter_resolution_keys()
        .map(|(module, name, descriptor)| {
            let symbol = name.to_string();
            let kind = name.kind();
            let canonical = module == descriptor.module
                && kind == descriptor.kind
                && symbol == descriptor.symbol;
            KeyRow {
                library: module.to_owned(),
                symbol,
                kind,
                canonical_library: descriptor.module.clone(),
                canonical_symbol: descriptor.symbol.clone(),
                canonical_kind: descriptor.kind,
                canonical,
                api_set: api_set(module),
            }
        })
        .collect();
    keys.sort_by(|left, right| {
        left.library
            .cmp(&right.library)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.symbol.cmp(&right.symbol))
    });

    let mut typed: Vec<&xenora_loader::runtime::ServiceMetadata> = SERVICES.iter().collect();
    typed.sort_by(|left, right| {
        left.library
            .cmp(right.library)
            .then_with(|| left.symbol.cmp(right.symbol))
    });

    let mut out = String::from("{\n  \"schema\": 1,\n  \"services\": [\n");
    for (index, service) in services.iter().enumerate() {
        service_row(&mut out, service, &typed_keys);
        if index + 1 != services.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ],\n  \"keys\": [\n");
    for (index, row) in keys.iter().enumerate() {
        key_row(&mut out, row);
        if index + 1 != keys.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ],\n  \"typed\": [\n");
    for (index, service) in typed.iter().enumerate() {
        typed_row(&mut out, service);
        if index + 1 != typed.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ]\n}\n");
    out
}

fn main() {
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    let mut registry = ImportRegistry::new();
    register_standard_services(&mut registry, state);
    print!("{}", snapshot(&registry));
}
