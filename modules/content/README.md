# content module

The `content` module generates message bodies for other modules. Its native crate `fps-module-content` implements the template functions `content.html`, `content.json`, `content.text`, `content.email`, and `content.binary` in Rust under decision D10, so a scenario renders a body with `{{ content.html(size=8192) }}` and no WebAssembly runtime. A module that calls the functions lists `{ module: content, version: ">=0.1" }` under `requires` in its manifest. The module speaks no protocol and has no scenarios or fixture capture, because the modules that call it, such as `http1`, `smtp`, `imap`, `pop3`, and `ftp`, carry the bodies on the wire.

## Provenance

Every generator is synthetic and holds no real personal data. The crate builds each body from word lists and sentence grammars that we wrote for this module, and it bundles no copyrighted text. It reads no file and opens no connection. The generators draw on nothing but their arguments and the session generator.

- Names, such as the owner of a JSON record or the sender of an email, join invented syllables and belong to no one.
- Email addresses end in `@example.test`, and links point to hosts under `example.test` or to paths on the same host. The `.test` domain is reserved by RFC 6761, and no real host serves it.
- Prose sentences make no claim about the world. A subject, a predicate, and an adverbial combine without regard to meaning, so the text reads like prose but says nothing.
- The binary files hold random bytes, gradients, or the same synthetic prose, and no photograph, logo, or document from anywhere.

The `provenance` block of `manifest.yaml` states `source: synthetic`, and a test checks the block and this section.

## Layout

The module follows the layout of the Modules section of the [Coding Standards](../../docs/wiki/Coding-Standards.md#modules).

| Path | Contents |
|---|---|
| `manifest.yaml` | Identity, the function catalog, and provenance |
| `native/` | The crate `fps-module-content` with the generators, the `TemplateFn` implementation `Content`, and the catalog `FUNCTIONS` |
| `tests/` | The catalog test and the generator tests, which include the duplicate payload ratio |

## Template functions

Every function takes `size` and `seed`. The output has at least `size` bytes, and at most about 1 KiB more, because a generator finishes the element that it is writing when it reaches the size, so a page ends with its footer and a JSON document ends with its closing brackets. The `bin` kind has exactly `size` bytes. A `size` below zero or above 16777216 fails the session with the reason `extension_invalid_argument`, as does an unknown `lang`, an unknown `kind`, or an argument of the wrong type. A size below the fixed part of a format, for example 100 for a PNG file, gives the smallest valid output.

A `seed` of zero or more fixes the output, so every session that calls the function with that seed gets the same bytes. A negative `seed`, the default, draws a seed from the session generator, so the bytes depend only on the run seed, the session id, and the call order, and a replay reproduces them while every session gets other content. Two functions called with the same seed use different random sequences.

| Function | Parameters with defaults | Result |
|---|---|---|
| `html` | `size=4096`, `seed=-1` | An HTML 5 page with a head, a header with a navigation list, a main element, and a footer. Sections of paragraphs, lists, tables, figures, and quotations fill the main element. |
| `json` | `size=2048`, `seed=-1` | A JSON document in the shape of a paged API response, with `status`, `request_id`, `page`, `links`, a `data` array of records, and `meta`. A record has an id, a name, an owner, a status, a score, tags, a timestamp, a URL, and a note. |
| `text` | `lang='en'`, `size=1024`, `seed=-1` | Plain prose in paragraphs of three to six sentences, ending with one line break |
| `email` | `lang='en'`, `size=1024`, `seed=-1` | A plain text body with a greeting, prose, a `Ref:` line, a closing, and a signature with an address under `example.test` |
| `binary` | `kind='png'`, `size=4096`, `seed=-1` | A file of the given kind |

`lang` takes `en`, `de`, `fr`, `es`, `it`, `ru`, or `zh`. The `text` and `email` functions return UTF-8, and `size` counts bytes, so a Russian or Chinese text holds fewer characters than an English one of the same size.

`kind` takes the following values.

| Kind | Content |
|---|---|
| `png` | An RGB image of random width with a gradient and noise. The deflate stream holds stored blocks. Chunk checksums and the Adler-32 trailer are correct, and any viewer opens the file. |
| `gif` | A GIF89a image with a random palette and random pixels, with a clear code every 200 pixels, so the image needs no table growth. Viewers open the file. |
| `jpeg` | A JFIF header, comment segments with random bytes, and the end marker. The marker structure is valid, but the file has no image scan, so a viewer shows nothing. |
| `pdf` | A one page PDF 1.4 file with lines of synthetic prose in Helvetica and a correct cross reference table |
| `zip` | An archive with one stored entry, `report-<hex>.txt`, that holds synthetic prose |
| `gzip` | A gzip member with stored deflate blocks that hold synthetic prose |
| `bin` | Random bytes, exactly `size` of them |

## Variety and the duplicate ratio

Each body draws many independent choices from the generator, such as the section order, the words, the numbers, and the identifiers, so two bodies rarely coincide. The test `duplicate_ratio_over_10000_session_bodies_is_at_most_one_percent` generates 10,000 bodies per generator type with the seed unset, and the sibling tests do the same with the seeds 0 to 9999 and with 512 byte bodies. A generator type is `html`, `json`, `text` and `email` in each language, and `binary` of each kind. The ratio is one minus the number of distinct bodies over the number of bodies, and it must stay at or below 0.01, the acceptance bound of MOD-03.

Small sizes limit the variety of a body. A 512 byte text holds about four sentences, which is why the test uses it as the lower end.

## Tests

```bash
make lint-mod-content test-mod-content
```

The target `lint-mod-content` validates the manifest with `tools/contracts/validate.py`. The target `test-mod-content` runs the unit tests, the catalog test, and the generator tests. The generator tests check the following.

1. The manifest catalog equals the native catalog.
2. The duplicate ratio stays within 1% over 10,000 bodies per generator type.
3. The same seed gives the same bytes, and another seed gives other bytes.
4. The output reaches `size` and stays within 1 KiB above it.
5. Every format parses: the HTML tags balance, the JSON document parses, and the PNG, GIF, JPEG, PDF, ZIP, and GZIP structures carry correct lengths, offsets, and checksums.
6. Addresses and links stay under `example.test`.
