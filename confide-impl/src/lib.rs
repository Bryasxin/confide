use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{Attribute, Expr, ItemStruct, Meta, parse_macro_input, spanned::Spanned};

enum FieldAnnotation {
    DefaultExpr(Expr),
    Optional,
}

fn extract_annotation(attrs: &[Attribute]) -> Option<FieldAnnotation> {
    attrs.iter().find_map(|attr| {
        if attr.path().is_ident("default") {
            match &attr.meta {
                // #[default = "val"]
                Meta::NameValue(nv) => Some(FieldAnnotation::DefaultExpr(nv.value.clone())),
                _ => None,
            }
        } else if attr.path().is_ident("optional") {
            Some(FieldAnnotation::Optional)
        } else {
            None
        }
    })
}

#[proc_macro_attribute]
pub fn confide(_attr: TokenStream, item: TokenStream) -> TokenStream {
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
        let annotation = extract_annotation(&field.attrs);

        let mut attrs_out: Vec<Attribute> = field
            .attrs
            .iter()
            .filter(|a| !(a.path().is_ident("default") || a.path().is_ident("optional")))
            .cloned()
            .collect();

        let fn_name = format_ident!("__confide_default_{}", field_name);
        let fn_path = format!("{}::{}", struct_name, fn_name);

        match annotation {
            Some(FieldAnnotation::DefaultExpr(expr)) => {
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
            Some(FieldAnnotation::Optional) => {
                attrs_out.push(syn::parse_quote! {
                    #[serde(default)]
                });

                default_fns.push(quote! {
                    #[allow(non_snake_case)]
                    fn #fn_name() -> #field_type {
                        ::core::default::Default::default()
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

    let default_impl = quote! {
        impl #impl_generics Default for #struct_name #type_generics #where_clause {
            fn default() -> Self {
                Self {
                    #(#default_fields)*
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
