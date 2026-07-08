use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Attribute, Expr, ItemStruct, Meta, parse::Parse, parse::ParseStream, parse_macro_input,
    spanned::Spanned,
};

struct ConfideArgs {
    no_default: bool,
    no_debug: bool,
}

impl Parse for ConfideArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut no_default = false;
        let mut no_debug = false;
        let vars = input.parse_terminated(syn::Ident::parse, syn::Token![,])?;
        for var in vars {
            if var == "no_default" {
                no_default = true;
            } else if var == "no_debug" {
                no_debug = true;
            } else {
                return Err(syn::Error::new(
                    var.span(),
                    format!("unknown confide argument: `{var}`"),
                ));
            }
        }
        Ok(ConfideArgs {
            no_default,
            no_debug,
        })
    }
}

impl Default for ConfideArgs {
    fn default() -> Self {
        ConfideArgs {
            no_default: false,
            no_debug: false,
        }
    }
}

enum FieldAnnotation {
    DefaultExpr(Expr),
    DefaultDefault,
    DurationExpr(String),
    BytesExpr(String),
}

struct FieldAttr {
    annotation: Option<FieldAnnotation>,
    secret: bool,
}

fn is_expr_path(expr: &Expr) -> bool {
    matches!(expr, Expr::Path(_))
}

fn extract_field_attr(attrs: &[Attribute]) -> syn::Result<FieldAttr> {
    for attr in attrs {
        if attr.path().is_ident("confide") {
            match &attr.meta {
                Meta::List(list) => {
                    let args: FieldConfideArgs = syn::parse2(list.tokens.clone())?;
                    let annotation = match args.default {
                        Some(FieldDefault::Expr(expr)) => Some(FieldAnnotation::DefaultExpr(expr)),
                        Some(FieldDefault::Bare) => Some(FieldAnnotation::DefaultDefault),
                        Some(FieldDefault::Duration(s)) => Some(FieldAnnotation::DurationExpr(s)),
                        Some(FieldDefault::Bytes(s)) => Some(FieldAnnotation::BytesExpr(s)),
                        None => None,
                    };
                    return Ok(FieldAttr {
                        annotation,
                        secret: args.secret,
                    });
                }
                Meta::Path(_) => {
                    return Err(syn::Error::new_spanned(
                        &attr.meta,
                        "bare `#[confide]` on a field is not supported; did you mean `#[confide(default)]`?",
                    ));
                }
                Meta::NameValue(_) => {
                    return Err(syn::Error::new_spanned(
                        &attr.meta,
                        "`#[confide = ...]` on a field is not supported; did you mean `#[confide(default = ...)]`?",
                    ));
                }
            }
        }
    }
    Ok(FieldAttr {
        annotation: None,
        secret: false,
    })
}

enum FieldDefault {
    Expr(Expr),
    Bare,
    Duration(String),
    Bytes(String),
}

struct FieldConfideArgs {
    default: Option<FieldDefault>,
    secret: bool,
}

impl Parse for FieldConfideArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut default = None;
        let mut secret = false;
        let metas = input.parse_terminated(Meta::parse, syn::Token![,])?;
        for meta in metas {
            match &meta {
                Meta::Path(p) if p.is_ident("default") => {
                    default = Some(FieldDefault::Bare);
                }
                Meta::Path(p) if p.is_ident("secret") => {
                    secret = true;
                }
                Meta::NameValue(nv) if nv.path.is_ident("default") => {
                    default = Some(FieldDefault::Expr(nv.value.clone()));
                }
                Meta::NameValue(nv) if nv.path.is_ident("default_duration") => {
                    if let Expr::Lit(lit) = &nv.value {
                        if let syn::Lit::Str(s) = &lit.lit {
                            default = Some(FieldDefault::Duration(s.value()));
                            continue;
                        }
                    }
                    return Err(syn::Error::new_spanned(
                        &nv.value,
                        "expected a string literal, e.g. default_duration = \"10s\"",
                    ));
                }
                Meta::NameValue(nv) if nv.path.is_ident("default_bytes") => {
                    if let Expr::Lit(lit) = &nv.value {
                        if let syn::Lit::Str(s) = &lit.lit {
                            default = Some(FieldDefault::Bytes(s.value()));
                            continue;
                        }
                    }
                    return Err(syn::Error::new_spanned(
                        &nv.value,
                        "expected a string literal, e.g. default_bytes = \"1MiB\"",
                    ));
                }
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "unexpected confide field argument",
                    ));
                }
            }
        }
        Ok(FieldConfideArgs { default, secret })
    }
}

#[proc_macro_attribute]
pub fn confide(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = if attr.is_empty() {
        ConfideArgs::default()
    } else {
        match syn::parse::<ConfideArgs>(attr) {
            Ok(args) => args,
            Err(e) => return e.to_compile_error().into(),
        }
    };
    let struct_input = parse_macro_input!(item as ItemStruct);
    let struct_name = &struct_input.ident;
    let struct_visibility = &struct_input.vis;
    let (impl_generics, type_generics, where_clause) = struct_input.generics.split_for_impl();
    let struct_outer_attrs: Vec<_> = struct_input
        .attrs
        .iter()
        .filter(|a| !a.path().is_ident("confide"))
        .collect();

    let fields = match &struct_input.fields {
        syn::Fields::Named(fields_name) => &fields_name.named,
        _ => {
            return syn::Error::new(
                struct_input.span(),
                "confide only supports structs with named fields",
            )
            .to_compile_error()
            .into();
        }
    };

    let mut default_fns = Vec::new();
    let mut field_outputs = Vec::new();
    let mut default_fields = Vec::new();
    let mut debug_fields: Vec<(syn::Ident, bool)> = Vec::new();

    for field in fields.iter() {
        let field_name = field.ident.as_ref().expect("named field");
        let field_visibility = &field.vis;
        let field_type = &field.ty;
        let field_attr = match extract_field_attr(&field.attrs) {
            Ok(a) => a,
            Err(e) => return e.to_compile_error().into(),
        };
        let annotation = field_attr.annotation;
        let is_secret = field_attr.secret;

        let mut attrs_out: Vec<Attribute> = field
            .attrs
            .iter()
            .filter(|a| !a.path().is_ident("confide"))
            .cloned()
            .collect();

        match annotation {
            Some(FieldAnnotation::DefaultExpr(expr)) => {
                if is_expr_path(&expr) {
                    let path_str = quote!(#expr).to_string();

                    attrs_out.push(syn::parse_quote! {
                        #[serde(default = #path_str)]
                    });

                    default_fields.push(quote! { #field_name: #expr(), });
                } else {
                    let fn_name = format_ident!("__confide_default_{}", field_name);
                    let fn_path = format!("{}::{}", struct_name, fn_name);

                    attrs_out.push(syn::parse_quote! {
                        #[serde(default = #fn_path)]
                    });

                    default_fns.push(quote! {
                        #[allow(non_snake_case)]
                        fn #fn_name() -> #field_type {
                            #expr
                        }
                    });

                    default_fields.push(quote! { #field_name: Self::#fn_name(), });
                }
            }
            Some(FieldAnnotation::DefaultDefault) => {
                attrs_out.push(syn::parse_quote! {
                    #[serde(default)]
                });

                default_fields.push(quote! { #field_name: ::core::default::Default::default(), });
            }
            Some(FieldAnnotation::DurationExpr(s)) => {
                let duration = match humantime::parse_duration(&s) {
                    Ok(d) => d,
                    Err(e) => {
                        return syn::Error::new(
                            field_name.span(),
                            format!("invalid duration: {e}"),
                        )
                        .to_compile_error()
                        .into();
                    }
                };
                let secs = duration.as_secs();
                let nanos = duration.subsec_nanos();
                let fn_name = format_ident!("__confide_default_{}", field_name);
                let fn_path = format!("{}::{}", struct_name, fn_name);

                attrs_out.push(syn::parse_quote! {
                    #[serde(with = "confide::humantime_serde")]
                });
                attrs_out.push(syn::parse_quote! {
                    #[serde(default = #fn_path)]
                });

                default_fns.push(quote! {
                    #[allow(non_snake_case)]
                    fn #fn_name() -> #field_type {
                        ::core::time::Duration::new(#secs, #nanos)
                    }
                });

                default_fields.push(quote! { #field_name: Self::#fn_name(), });
            }
            Some(FieldAnnotation::BytesExpr(s)) => {
                let bs: bytesize::ByteSize = match s.parse() {
                    Ok(v) => v,
                    Err(e) => {
                        return syn::Error::new(
                            field_name.span(),
                            format!("invalid byte size: {e}"),
                        )
                        .to_compile_error()
                        .into();
                    }
                };
                let bytes = bs.as_u64();
                let fn_name = format_ident!("__confide_default_{}", field_name);
                let fn_path = format!("{}::{}", struct_name, fn_name);

                let ser_fn_name = format_ident!("__confide_serialize_bytes_{}", field_name);
                let de_fn_name = format_ident!("__confide_deserialize_bytes_{}", field_name);
                let ser_path = format!("{}::{}", struct_name, ser_fn_name);
                let de_path = format!("{}::{}", struct_name, de_fn_name);

                attrs_out.push(syn::parse_quote! {
                    #[serde(
                        serialize_with = #ser_path,
                        deserialize_with = #de_path
                    )]
                });
                attrs_out.push(syn::parse_quote! {
                    #[serde(default = #fn_path)]
                });

                default_fns.push(quote! {
                    #[allow(non_snake_case)]
                    fn #fn_name() -> #field_type {
                        ::core::convert::TryInto::<#field_type>::try_into(#bytes)
                            .expect("default_bytes value out of range for field type")
                    }
                });
                default_fns.push(quote! {
                    #[allow(non_snake_case, unused)]
                    fn #ser_fn_name<__S: ::serde::Serializer>(
                        val: &#field_type,
                        s: __S,
                    ) -> ::core::result::Result<__S::Ok, __S::Error> {
                        let bytes: u64 = ::core::convert::TryInto::<u64>::try_into(*val)
                            .map_err(|_| {
                                ::serde::ser::Error::custom(
                                    ::core::format_args!(
                                        "byte size field value out of range for u64"
                                    ),
                                )
                            })?;
                        ::serde::Serialize::serialize(
                            &::confide::bytesize::ByteSize::b(bytes)
                                .display()
                                .iec()
                                .to_string(),
                            s,
                        )
                    }
                });
                default_fns.push(quote! {
                    #[allow(non_snake_case, unused)]
                    fn #de_fn_name<'__de, __D: ::serde::Deserializer<'__de>>(
                        d: __D,
                    ) -> ::core::result::Result<#field_type, __D::Error> {
                        struct __ConfideBytesVisitor;
                        impl<'__de2> ::serde::de::Visitor<'__de2> for __ConfideBytesVisitor {
                            type Value = u64;
                            fn expecting(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
                                f.write_str("a byte size string like '1MiB' or an integer")
                            }
                            fn visit_u64<__E: ::serde::de::Error>(
                                self,
                                v: u64,
                            ) -> ::core::result::Result<u64, __E> {
                                Ok(v)
                            }
                            fn visit_i64<__E: ::serde::de::Error>(
                                self,
                                v: i64,
                            ) -> ::core::result::Result<u64, __E> {
                                u64::try_from(v).map_err(|_| {
                                    __E::invalid_value(
                                        ::serde::de::Unexpected::Signed(v),
                                        &"non-negative integer",
                                    )
                                })
                            }
                            fn visit_str<__E: ::serde::de::Error>(
                                self,
                                v: &str,
                            ) -> ::core::result::Result<u64, __E> {
                                v.parse::<::confide::bytesize::ByteSize>()
                                    .map(|bs| bs.as_u64())
                                    .map_err(|_| {
                                        __E::invalid_value(::serde::de::Unexpected::Str(v), &self)
                                    })
                            }
                        }
                        let val: u64 = if d.is_human_readable() {
                            d.deserialize_any(__ConfideBytesVisitor)?
                        } else {
                            d.deserialize_u64(__ConfideBytesVisitor)?
                        };
                        #field_type::try_from(val).map_err(|_| {
                            ::serde::de::Error::invalid_value(
                                ::serde::de::Unexpected::Unsigned(val),
                                &concat!(
                                    "a byte size value fitting in ",
                                    ::core::stringify!(#field_type),
                                ),
                            )
                        })
                    }
                });

                default_fields.push(quote! { #field_name: Self::#fn_name(), });
            }
            None => {}
        }

        debug_fields.push((field_name.clone(), is_secret));

        field_outputs.push(quote! {
            #(#attrs_out)*
            #field_visibility #field_name: #field_type,
        });
    }

    let default_impl = if args.no_default {
        quote! {}
    } else {
        quote! {
            impl #impl_generics Default for #struct_name #type_generics #where_clause {
                fn default() -> Self {
                    Self {
                        #(#default_fields)*
                    }
                }
            }
        }
    };

    let debug_impl = if args.no_debug {
        quote! {}
    } else {
        let field_entries = debug_fields.iter().map(|(name, secret)| {
            let name_str = name.to_string();
            if *secret {
                quote! { .field(#name_str, &"***") }
            } else {
                quote! { .field(#name_str, &self.#name) }
            }
        });
        quote! {
            impl #impl_generics ::core::fmt::Debug for #struct_name #type_generics #where_clause {
                fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                    f.debug_struct(stringify!(#struct_name))
                        #(#field_entries)*
                        .finish()
                }
            }
        }
    };

    let expanded = quote! {
        #(#struct_outer_attrs)*
        #struct_visibility struct #struct_name #type_generics #where_clause {
            #(#field_outputs)*
        }

        impl #impl_generics #struct_name #type_generics #where_clause {
            #(#default_fns)*
        }

        #default_impl
        #debug_impl
    };
    expanded.into()
}
