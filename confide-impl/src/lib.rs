use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Attribute, Expr, ItemStruct, Meta, parse::Parse, parse::ParseStream, parse_macro_input,
    spanned::Spanned,
};

struct ConfideArgs {
    no_default: bool,
}

impl Parse for ConfideArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut no_default = false;
        let vars = input.parse_terminated(syn::Ident::parse, syn::Token![,])?;
        for var in vars {
            if var == "no_default" {
                no_default = true;
            } else {
                return Err(syn::Error::new(
                    var.span(),
                    format!("unknown confide argument: `{var}`"),
                ));
            }
        }
        Ok(ConfideArgs { no_default })
    }
}

impl Default for ConfideArgs {
    fn default() -> Self {
        ConfideArgs { no_default: false }
    }
}

enum FieldAnnotation {
    DefaultExpr(Expr),
    DefaultDefault,
    DurationExpr(String),
}

fn is_expr_path(expr: &Expr) -> bool {
    matches!(expr, Expr::Path(_))
}

fn extract_annotation(attrs: &[Attribute]) -> syn::Result<Option<FieldAnnotation>> {
    for attr in attrs {
        if attr.path().is_ident("default") {
            match &attr.meta {
                Meta::Path(_) => return Ok(Some(FieldAnnotation::DefaultDefault)),
                Meta::NameValue(nv) => {
                    return Ok(Some(FieldAnnotation::DefaultExpr(nv.value.clone())));
                }
                _ => continue,
            }
        } else if attr.path().is_ident("duration") {
            match &attr.meta {
                Meta::NameValue(nv) => {
                    if let Expr::Lit(lit) = &nv.value {
                        if let syn::Lit::Str(s) = &lit.lit {
                            return Ok(Some(FieldAnnotation::DurationExpr(s.value())));
                        }
                    }
                    return Err(syn::Error::new_spanned(
                        &nv.value,
                        "expected a string literal, e.g. #[duration = \"10s\"]",
                    ));
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        attr,
                        "expected a string literal, e.g. #[duration = \"10s\"]",
                    ));
                }
            }
        }
    }
    Ok(None)
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
    let struct_outer_attrs = &struct_input.attrs;

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

    for field in fields.iter() {
        let field_name = field.ident.as_ref().expect("named field");
        let field_visibility = &field.vis;
        let field_type = &field.ty;
        let annotation = match extract_annotation(&field.attrs) {
            Ok(a) => a,
            Err(e) => return e.to_compile_error().into(),
        };

        let mut attrs_out: Vec<Attribute> = field
            .attrs
            .iter()
            .filter(|a| !(a.path().is_ident("default") || a.path().is_ident("duration")))
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
            None => {}
        }

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

    let expanded = quote! {
        #(#struct_outer_attrs)*
        #struct_visibility struct #struct_name #type_generics #where_clause {
            #(#field_outputs)*
        }

        impl #impl_generics #struct_name #type_generics #where_clause {
            #(#default_fns)*
        }

        #default_impl
    };
    expanded.into()
}
