use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    parse_macro_input, parse_quote, Data, DeriveInput, Fields, GenericParam, Generics, Ident,
    Index, LitStr, Token,
};

#[proc_macro_derive(Arg, attributes(struct_args_match))]
pub fn derive_args(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    // Parse the input tokens into a syntax tree.
    let input = parse_macro_input!(input as DeriveInput);

    // Used in the quasi-quotation below as `#name`.
    let name = input.ident;

    let generics = add_trait_bounds(input.generics);
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    // Generate an expression to sum up the heap size of each field.
    let body = arg_parse(&input.data);

    let expanded = quote! {
        // The generated impl.
        impl #impl_generics struct_args::Arg for #name #ty_generics #where_clause {
            fn parse_args(args: &mut Vec<String>) -> anyhow::Result<Self> {
                Ok(#body)
            }
        }
    };

    // Hand the output tokens back to the compiler.
    proc_macro::TokenStream::from(expanded)
}

fn add_trait_bounds(mut generics: Generics) -> Generics {
    for param in &mut generics.params {
        if let GenericParam::Type(ref mut type_param) = *param {
            type_param.bounds.push(parse_quote!(struct_args::Arg));
        }
    }
    generics
}

fn parse_fields(fields: &Fields) -> TokenStream {
    match fields {
        Fields::Named(ref fields) => {
            let recurse = fields.named.iter().map(|f| {
                let name = &f.ident;
                quote_spanned! {f.span()=>
                    #name: struct_args::Arg::parse_args(args)?
                }
            });
            quote! {
                {#(#recurse, )*}
            }
        }
        Fields::Unnamed(ref fields) => {
            let recurse = fields.unnamed.iter().enumerate().map(|(i, f)| {
                let _index = Index::from(i);
                quote_spanned! {f.span()=>
                    struct_args::Arg::parse_args(args)?
                }
            });
            quote! {
                (#(#recurse, )*)
            }
        }
        Fields::Unit => {
            quote!()
        }
    }
}

enum Item {
    LitStr(LitStr),
    Ident(Ident),
}

impl Parse for Item {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let lookahead = input.lookahead1();
        if lookahead.peek(LitStr) {
            input.parse().map(Item::LitStr)
        } else if lookahead.peek(Ident) {
            input.parse().map(Item::Ident)
        } else {
            Err(lookahead.error())
        }
    }
}

fn arg_parse(data: &Data) -> TokenStream {
    match *data {
        Data::Struct(ref data) => {
            let data = parse_fields(&data.fields);
            quote! {
                Self#data
            }
        }
        Data::Enum(ref data) => {
            let mut valid_names = vec![];
            let recurse = data
                .variants
                .iter()
                .map(|v| {
                    let mut names = vec![];
                    let mut no_default = false;
                    for item in v
                        .attrs
                        .iter()
                        .filter(|a| a.path.is_ident("struct_args_match"))
                        .flat_map(|a| {
                            a.parse_args_with(Punctuated::<Item, Token![,]>::parse_terminated)
                                .unwrap()
                        })
                    {
                        match item {
                            Item::LitStr(name) => names.push(name.value()),
                            Item::Ident(ident) => {
                                if ident == "ND" {
                                    no_default = true;
                                }
                            }
                        }
                    }
                    let name = &v.ident;
                    let data = parse_fields(&v.fields);
                    if !no_default {
                        names.push(name.to_string().to_lowercase())
                    }
                    let out = quote_spanned! {v.span()=>
                        #( #names )|* => Self::#name#data
                    };
                    valid_names.extend(names);
                    out
                })
                .collect::<Vec<_>>();
            let names = valid_names.iter().fold(String::new(), |x, y| x + y + ", ");
            quote!(
                match args.pop().ok_or_else(|| anyhow::Error::msg(format!("No argument provided. Valid: {}", #names)))?.as_str() {
                    #(#recurse, )*
                    s => anyhow::bail!("Invalid Option {}. Valid: {}", s, #names)
                }
            )
        }
        Data::Union(_) => unimplemented!(),
    }
}
