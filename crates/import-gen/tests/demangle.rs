// SPDX-License-Identifier: GPL-3.0-or-later

//! Decorated names produced by an independent mangler: Apple clang 21 with
//! `-target x86_64-pc-windows-msvc -S` on the declarations quoted beside each
//! case. Expected strings restate those declarations in `undname` style; they
//! are not taken from the decoder.

use xenora_import_gen::demangle;

#[test]
fn clang_msvc_manglings_decode_to_their_declarations() {
    for (decorated, expected) in [
        // void foo();
        ("?foo@@YAXXZ", "void __cdecl foo(void)"),
        // int foo2(int);
        ("?foo2@@YAHH@Z", "int __cdecl foo2(int)"),
        // void f(const char*);
        ("?f@@YAXPEBD@Z", "void __cdecl f(char const * __ptr64)"),
        // void fv(int, ...);
        ("?fv@@YAXHZZ", "void __cdecl fv(int,...)"),
        // bool g(long long, long long);
        ("?g@@YA_N_J0@Z", "bool __cdecl g(__int64,__int64)"),
        // namespace std { void _Xlength_error(const char*); }
        (
            "?_Xlength_error@std@@YAXPEBD@Z",
            "void __cdecl std::_Xlength_error(char const * __ptr64)",
        ),
        // struct Foo { void bar(int); };
        (
            "?bar@Foo@@QEAAXH@Z",
            "public: void __cdecl Foo::bar(int) __ptr64",
        ),
        // struct Foo { int get() const; };
        (
            "?get@Foo@@QEBAHXZ",
            "public: int __cdecl Foo::get(void)const __ptr64",
        ),
        // struct Foo { Foo(); };
        ("??0Foo@@QEAA@XZ", "public: __cdecl Foo::Foo(void) __ptr64"),
        // struct Foo { static Foo* make(); };
        (
            "?make@Foo@@SAPEAU1@XZ",
            "public: static struct Foo * __ptr64 __cdecl Foo::make(void)",
        ),
        // void use(Foo&);
        (
            "?use@@YAXAEAUFoo@@@Z",
            "void __cdecl use(struct Foo & __ptr64)",
        ),
        // namespace ns { int x; }
        ("?x@ns@@3HA", "int ns::x"),
        // Global operator new(size_t) and sized operator delete(void*, size_t).
        (
            "??2@YAPEAX_K@Z",
            "void * __ptr64 __cdecl operator new(unsigned __int64)",
        ),
        (
            "??3@YAXPEAX_K@Z",
            "void __cdecl operator delete(void * __ptr64,unsigned __int64)",
        ),
        // void gs(std::basic_string<char, std::char_traits<char>, std::allocator<char>>&);
        (
            "?gs@@YAXAEAV?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@@Z",
            "void __cdecl gs(class std::basic_string<char,struct std::char_traits<char>,class std::allocator<char> > & __ptr64)",
        ),
        // void h(std::array<int, 16>);
        (
            "?h@@YAXV?$array@H$0BA@@std@@@Z",
            "void __cdecl h(class std::array<int,16>)",
        ),
    ] {
        assert_eq!(
            demangle(decorated).as_deref(),
            Some(expected),
            "{decorated}"
        );
    }
}

#[test]
fn unsupported_clang_manglings_are_reported_rather_than_guessed() {
    // void (*keep[])(): function pointer types are outside the supported subset.
    assert_eq!(demangle("?keep@@3PAP6AXXZA"), None);
    // A compiler-generated cleanup funclet nested in another function's scope.
    assert_eq!(demangle("?dtor$2@?0??use@@YAXAEAUFoo@@@Z@4HA"), None);
}

#[test]
fn manifests_show_undecorated_names_for_unclassified_cxx_imports() {
    use xenora_import_gen::{ImportRecord, Manifest, SymbolDb};
    let record = |symbol: &str| ImportRecord {
        module: "msvcp140.dll".to_owned(),
        symbol: symbol.to_owned(),
        ordinal: None,
        iat_rva: 0x2000,
    };
    let manifest = Manifest::build(
        &[
            record("?_Xlength_error@std@@YAXPEBD@Z"),
            record("?keep@@3PAP6AXXZA"),
        ],
        &SymbolDb::builtin(),
    );
    let markdown = manifest.render_markdown();
    assert!(markdown.contains(
        "- `msvcp140.dll!?_Xlength_error@std@@YAXPEBD@Z` (`void __cdecl std::_Xlength_error(char const * __ptr64)`) —"
    ));
    assert!(markdown.contains("- `msvcp140.dll!?keep@@3PAP6AXXZA` —"));
}
