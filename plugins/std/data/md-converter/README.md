# Usage
Takes a json of `{ "fileType": "pdf", "b64Bytes": "ab7a7b...."}` where the file type tells us the extension which dictates the converter to use
and `b64Bytes` is the base64 encoded bytes of the file.

You must have [`marker`](https://github.com/VikParuchuri/marker) installed:
```
python3 -m pip install marker-pdf\[full]
```
