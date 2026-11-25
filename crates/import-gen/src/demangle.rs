// SPDX-License-Identifier: GPL-3.0-or-later

//! Decoding of Microsoft Visual C++ decorated names, for diagnostics.
//!
//! Written from the publicly described decoration scheme: a leading `?`,
//! `@`-terminated name fragments listed innermost first, single-digit name and
//! type back-references, an access and storage code, a calling convention,
//! the return type and the parameter list. Output follows the style of
//! Microsoft's `undname` tool. Anything outside the supported subset returns
//! `None` instead of a guess, so callers fall back to the decorated name.

const MAX_DEPTH: usize = 16;

/// The undecorated form of `symbol`, or `None` when it is not a decorated
/// C++ name or uses an unsupported construct.
pub fn demangle(symbol: &str) -> Option<String> {
    let mut parser = Parser {
        input: symbol.as_bytes(),
        position: 0,
        names: Vec::new(),
        types: Vec::new(),
        depth: 0,
    };
    let result = parser.symbol()?;
    (parser.position == parser.input.len()).then_some(result)
}

/// A special name introduced by `?` and a code inside the qualified name.
enum Special {
    Constructor,
    Destructor,
    Operator(&'static str),
    Vftable,
}

struct Parser<'a> {
    input: &'a [u8],
    position: usize,
    names: Vec<String>,
    types: Vec<String>,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.input.get(self.position).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.position += 1;
        Some(byte)
    }

    fn eat(&mut self, byte: u8) -> bool {
        let found = self.peek() == Some(byte);
        if found {
            self.position += 1;
        }
        found
    }

    fn eat_str(&mut self, text: &str) -> bool {
        let found = self.input[self.position..].starts_with(text.as_bytes());
        if found {
            self.position += text.len();
        }
        found
    }

    fn symbol(&mut self) -> Option<String> {
        if !self.eat(b'?') {
            return None;
        }
        let (name, special) = self.qualified_name(true)?;
        match self.peek()? {
            b'0'..=b'4' => self.variable(&name),
            b'6' => {
                self.next();
                let qualifier = cv(self.next()?)?;
                self.eat(b'@');
                if !matches!(special, Some(Special::Vftable)) {
                    return None;
                }
                Some(format!("{} {name}", qualifier.trim()))
            }
            _ => self.function(&name, special.as_ref()),
        }
    }

    fn variable(&mut self, name: &str) -> Option<String> {
        let access = match self.next()? {
            b'0' => "private: static ",
            b'1' => "protected: static ",
            b'2' => "public: static ",
            _ => "",
        };
        let kind = self.data_type()?;
        self.eat(b'E');
        let qualifier = cv(self.next()?)?;
        Some(format!("{access}{kind}{qualifier} {name}"))
    }

    fn function(&mut self, name: &str, special: Option<&Special>) -> Option<String> {
        let (access, member) = match self.next()? {
            b'A' | b'B' => ("private: ", true),
            b'C' | b'D' => ("private: static ", false),
            b'E' | b'F' => ("private: virtual ", true),
            b'I' | b'J' => ("protected: ", true),
            b'K' | b'L' => ("protected: static ", false),
            b'M' | b'N' => ("protected: virtual ", true),
            b'Q' | b'R' => ("public: ", true),
            b'S' | b'T' => ("public: static ", false),
            b'U' | b'V' => ("public: virtual ", true),
            b'Y' | b'Z' => ("", false),
            _ => return None,
        };
        let mut this = String::new();
        if member {
            let pointer64 = self.eat(b'E');
            this.push_str(cv(self.next()?)?.trim_start());
            if pointer64 {
                this.push_str(" __ptr64");
            }
        }
        let convention = match self.next()? {
            b'A' | b'B' => "__cdecl",
            b'C' | b'D' => "__pascal",
            b'E' | b'F' => "__thiscall",
            b'G' | b'H' => "__stdcall",
            b'I' | b'J' => "__fastcall",
            b'Q' => "__vectorcall",
            _ => return None,
        };
        let returns = if self.eat(b'@') {
            if !matches!(special, Some(Special::Constructor | Special::Destructor)) {
                return None;
            }
            String::new()
        } else {
            if self.eat(b'?') {
                cv(self.next()?)?;
            }
            format!("{} ", self.data_type()?)
        };
        let parameters = self.parameters()?;
        if !self.eat(b'Z') {
            return None;
        }
        Some(format!(
            "{access}{returns}{convention} {name}({parameters}){this}"
        ))
    }

    fn parameters(&mut self) -> Option<String> {
        if self.eat(b'X') {
            return Some("void".to_owned());
        }
        let mut parameters = Vec::new();
        loop {
            match self.peek()? {
                b'@' => {
                    self.next();
                    break;
                }
                b'Z' => {
                    self.next();
                    parameters.push("...".to_owned());
                    break;
                }
                _ => {
                    let start = self.position;
                    let kind = self.data_type()?;
                    if self.position - start > 1 && self.types.len() < 10 {
                        self.types.push(kind.clone());
                    }
                    parameters.push(kind);
                }
            }
        }
        Some(parameters.join(","))
    }

    /// Fragments up to `@`, joined outermost first.
    fn qualified_name(&mut self, allow_special: bool) -> Option<(String, Option<Special>)> {
        let mut special = None;
        let mut fragments = Vec::new();
        if allow_special
            && self.peek() == Some(b'?')
            && self.input.get(self.position + 1) != Some(&b'$')
        {
            self.next();
            special = Some(self.special()?);
        } else {
            fragments.push(self.fragment()?);
        }
        while !self.eat(b'@') {
            fragments.push(self.fragment()?);
        }
        fragments.reverse();
        let scope = fragments.join("::");
        let name = match &special {
            None => scope,
            Some(special) => {
                let class = || Some(fragments.last()?.split('<').next()?.to_owned());
                let leaf = match special {
                    Special::Constructor => class()?,
                    Special::Destructor => format!("~{}", class()?),
                    Special::Operator(name) => name.to_string(),
                    Special::Vftable => "`vftable'".to_owned(),
                };
                if scope.is_empty() {
                    leaf
                } else {
                    format!("{scope}::{leaf}")
                }
            }
        };
        Some((name, special))
    }

    fn special(&mut self) -> Option<Special> {
        Some(match self.next()? {
            b'0' => Special::Constructor,
            b'1' => Special::Destructor,
            b'2' => Special::Operator("operator new"),
            b'3' => Special::Operator("operator delete"),
            b'4' => Special::Operator("operator="),
            b'8' => Special::Operator("operator=="),
            b'9' => Special::Operator("operator!="),
            b'A' => Special::Operator("operator[]"),
            b'R' => Special::Operator("operator()"),
            b'_' => match self.next()? {
                b'7' => Special::Vftable,
                b'U' => Special::Operator("operator new[]"),
                b'V' => Special::Operator("operator delete[]"),
                _ => return None,
            },
            _ => return None,
        })
    }

    fn fragment(&mut self) -> Option<String> {
        if let Some(digit @ b'0'..=b'9') = self.peek() {
            self.next();
            return self.names.get(usize::from(digit - b'0')).cloned();
        }
        let fragment = if self.eat_str("?$") {
            self.template()?
        } else {
            self.identifier()?
        };
        if self.names.len() < 10 && !self.names.contains(&fragment) {
            self.names.push(fragment.clone());
        }
        Some(fragment)
    }

    fn identifier(&mut self) -> Option<String> {
        let start = self.position;
        while self.peek()? != b'@' {
            let byte = self.next()?;
            if !(byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$') {
                return None;
            }
        }
        let text = std::str::from_utf8(&self.input[start..self.position]).ok()?;
        self.next();
        (!text.is_empty()).then(|| text.to_owned())
    }

    /// `name@arguments@`, with back-references local to the argument list.
    fn template(&mut self) -> Option<String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return None;
        }
        let outer_names = std::mem::take(&mut self.names);
        let outer_types = std::mem::take(&mut self.types);
        let name = self.identifier();
        if let Some(name) = &name {
            self.names.push(name.clone());
        }
        let mut arguments = Vec::new();
        let result = (|| {
            let name = name?;
            while !self.eat(b'@') {
                let start = self.position;
                let argument = if self.eat(b'$') {
                    if !self.eat(b'0') {
                        return None;
                    }
                    self.number()?.to_string()
                } else {
                    self.data_type()?
                };
                if self.position - start > 1 && self.types.len() < 10 {
                    self.types.push(argument.clone());
                }
                arguments.push(argument);
            }
            let joined = arguments.join(",");
            let close = if joined.ends_with('>') { " >" } else { ">" };
            Some(format!("{name}<{joined}{close}"))
        })();
        self.names = outer_names;
        self.types = outer_types;
        self.depth -= 1;
        result
    }

    /// `0`–`9` encode 1–10; otherwise hexadecimal digits `A`–`P` end with `@`.
    fn number(&mut self) -> Option<i64> {
        let negative = self.eat(b'?');
        let value = match self.next()? {
            digit @ b'0'..=b'9' => i64::from(digit - b'0') + 1,
            b'@' => 0,
            mut digit => {
                let mut value: i64 = 0;
                loop {
                    if !(b'A'..=b'P').contains(&digit) {
                        return None;
                    }
                    value = value
                        .checked_mul(16)?
                        .checked_add(i64::from(digit - b'A'))?;
                    digit = self.next()?;
                    if digit == b'@' {
                        break value;
                    }
                }
            }
        };
        Some(if negative { -value } else { value })
    }

    fn data_type(&mut self) -> Option<String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return None;
        }
        let result = self.data_type_inner();
        self.depth -= 1;
        result
    }

    fn data_type_inner(&mut self) -> Option<String> {
        let code = self.next()?;
        let primitive = match code {
            b'X' => "void",
            b'C' => "signed char",
            b'D' => "char",
            b'E' => "unsigned char",
            b'F' => "short",
            b'G' => "unsigned short",
            b'H' => "int",
            b'I' => "unsigned int",
            b'J' => "long",
            b'K' => "unsigned long",
            b'M' => "float",
            b'N' => "double",
            b'O' => "long double",
            b'0'..=b'9' => return self.types.get(usize::from(code - b'0')).cloned(),
            b'_' => match self.next()? {
                b'J' => "__int64",
                b'K' => "unsigned __int64",
                b'N' => "bool",
                b'W' => "wchar_t",
                b'S' => "char16_t",
                b'U' => "char32_t",
                b'Q' => "char8_t",
                _ => return None,
            },
            b'V' => return Some(format!("class {}", self.qualified_name(false)?.0)),
            b'U' => return Some(format!("struct {}", self.qualified_name(false)?.0)),
            b'T' => return Some(format!("union {}", self.qualified_name(false)?.0)),
            b'W' => {
                if !self.eat(b'4') {
                    return None;
                }
                return Some(format!("enum {}", self.qualified_name(false)?.0));
            }
            b'P' | b'Q' | b'R' | b'S' => return self.indirection("*", pointer_cv(code)),
            b'A' => return self.indirection("&", ""),
            b'B' => return self.indirection("&", " volatile"),
            b'$' => {
                if self.eat_str("$Q") {
                    return self.indirection("&&", "");
                }
                if self.eat_str("$T") {
                    return Some("std::nullptr_t".to_owned());
                }
                return None;
            }
            _ => return None,
        };
        Some(primitive.to_owned())
    }

    /// A pointer or reference: an optional `E` for `__ptr64`, the pointee's
    /// qualifiers, then the pointee.
    fn indirection(&mut self, symbol: &str, own: &str) -> Option<String> {
        let pointer64 = self.eat(b'E');
        let pointee_cv = cv(self.next()?)?;
        let pointee = self.data_type()?;
        let width = if pointer64 { " __ptr64" } else { "" };
        Some(format!("{pointee}{pointee_cv} {symbol}{own}{width}"))
    }
}

fn cv(code: u8) -> Option<&'static str> {
    Some(match code {
        b'A' => "",
        b'B' => " const",
        b'C' => " volatile",
        b'D' => " const volatile",
        _ => return None,
    })
}

fn pointer_cv(code: u8) -> &'static str {
    match code {
        b'Q' => " const",
        b'R' => " volatile",
        b'S' => " const volatile",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::demangle;

    #[test]
    fn decodes_global_functions_and_operators() {
        for (decorated, expected) in [
            ("?foo@@YAXXZ", "void __cdecl foo(void)"),
            ("?foo@@YAHH@Z", "int __cdecl foo(int)"),
            ("?f@@YAXPEBD@Z", "void __cdecl f(char const * __ptr64)"),
            ("?f@@YAXHZZ", "void __cdecl f(int,...)"),
            (
                "??2@YAPEAX_K@Z",
                "void * __ptr64 __cdecl operator new(unsigned __int64)",
            ),
            (
                "??3@YAXPEAX@Z",
                "void __cdecl operator delete(void * __ptr64)",
            ),
            (
                "??_U@YAPEAX_K@Z",
                "void * __ptr64 __cdecl operator new[](unsigned __int64)",
            ),
            (
                "?_Xlength_error@std@@YAXPEBD@Z",
                "void __cdecl std::_Xlength_error(char const * __ptr64)",
            ),
            ("?g@@YA_N_J0@Z", "bool __cdecl g(__int64,__int64)"),
        ] {
            assert_eq!(
                demangle(decorated).as_deref(),
                Some(expected),
                "{decorated}"
            );
        }
    }

    #[test]
    fn decodes_members_constructors_and_data() {
        for (decorated, expected) in [
            (
                "?bar@Foo@@QEAAXH@Z",
                "public: void __cdecl Foo::bar(int) __ptr64",
            ),
            (
                "?get@Foo@@QEBAHXZ",
                "public: int __cdecl Foo::get(void)const __ptr64",
            ),
            ("??0Foo@@QEAA@XZ", "public: __cdecl Foo::Foo(void) __ptr64"),
            (
                "??1Foo@@UEAA@XZ",
                "public: virtual __cdecl Foo::~Foo(void) __ptr64",
            ),
            (
                "?make@Foo@@SAPEAV1@XZ",
                "public: static class Foo * __ptr64 __cdecl Foo::make(void)",
            ),
            ("?x@ns@@3HA", "int ns::x"),
            ("?count@Foo@@2HB", "public: static int const Foo::count"),
            ("??_7type_info@@6B@", "const type_info::`vftable'"),
        ] {
            assert_eq!(
                demangle(decorated).as_deref(),
                Some(expected),
                "{decorated}"
            );
        }
    }

    #[test]
    fn decodes_templates_and_back_references() {
        assert_eq!(
            demangle("?g@@YAXAEAV?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@@Z")
                .as_deref(),
            Some(
                "void __cdecl g(class std::basic_string<char,struct std::char_traits<char>,class std::allocator<char> > & __ptr64)"
            )
        );
        assert_eq!(
            demangle("?h@@YAXV?$array@H$0BA@@std@@@Z").as_deref(),
            Some("void __cdecl h(class std::array<int,16>)")
        );
    }

    #[test]
    fn rejects_undecorated_and_unsupported_names() {
        for symbol in [
            "malloc",
            "",
            "?",
            "?foo@@",
            "?foo@@YAXXZtrailing",
            "??_R0?AVtype_info@@@8",
            "?f@@YAXP6AXXZ@Z",
            &format!("?f@@YAX{}@Z", "PEA".repeat(40) + "H"),
        ] {
            assert_eq!(demangle(symbol), None, "{symbol}");
        }
    }
}
