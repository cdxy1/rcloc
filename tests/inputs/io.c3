// https://github.com/c3lang/c3c/blob/master/lib/std/io/io.c3  (partial saved here)
// Copyright (c) 2021-2025 Christoffer Lerno. All rights reserved.
// Use of this source code is governed by the MIT license
// a copy of which can be found in the LICENSE_STDLIB file.
module std::io;
import libc;

enum Seek
{
	SET,
	CURSOR,
	END
}

faultdef
	ALREADY_EXISTS,
	BUSY,
	CANNOT_READ_DIR,
	DIR_NOT_EMPTY,
	PARENT_DIR_MISSING,
	EOF,
	FILE_CANNOT_DELETE,
	FILE_IS_DIR,
	FILE_IS_PIPE,
	FILE_NOT_DIR,
	FILE_NOT_FOUND,
	FILE_NOT_VALID,
	GENERAL_ERROR,
	ILLEGAL_ARGUMENT,
	INCOMPLETE_WRITE,
	INTERRUPTED,
	INVALID_POSITION,
	INVALID_PUSHBACK,
	NAME_TOO_LONG,
	NOT_SEEKABLE,
	NO_PERMISSION,
	OUT_OF_SPACE,
	OVERFLOW,
	PATH_COULD_NOT_BE_FOUND,
	READ_ONLY,
	SYMLINK_FAILED,
	TOO_MANY_DESCRIPTORS,
	UNEXPECTED_EOF,
	UNKNOWN_ERROR,
	UNSUPPORTED_OPERATION,
	WOULD_BLOCK;


<*
 Read from a stream (default is stdin) to the next "\n"
 or to the end of the stream, whatever comes first.
 "\r" will be filtered from the String.

 @param [&inout] allocator : "The allocator used to allocate the read string."
 @param stream : `The stream to read from.`
 @param limit : `Optionally limits the amount of bytes to read in a single line. Will NOT discard the remaining line data.`
 @require @is_not_instream_if_ptr(stream) : "The value for 'stream' should have been passed as a pointer and not as a value, please add '&'."
 @require @is_instream(stream) : `Make sure that the stream is actually an InStream.`
 @param [inout] allocator : `the allocator to use.`
 @return `The string containing the data read.`
*>
macro String? readline(Allocator allocator, stream = io::stdin(), usz limit = 0)
{
	return readline_impl{$typeof(stream)}(allocator, stream, limit);
}

<*
 Reads a string, see `readline`, except the it is allocated
 on the temporary allocator and does not need to be freed.

 @param stream : `The stream to read from.`
 @param limit : `Optionally limits the amount of bytes to read in a single line. Will NOT discard the remaining line data.`
 @require @is_not_instream_if_ptr(stream) : "The value for 'stream' should have been passed as a pointer and not as a value, please add '&'."
 @require @is_instream(stream) : `The stream must implement InStream.`
 @return `The temporary string containing the data read.`
*>
macro String? treadline(stream = io::stdin(), usz limit = 0)
{
	return readline(tmem, stream, limit) @inline;
}

fn String? readline_impl(Allocator allocator, Stream stream, usz limit) <Stream> @private
{
	if (allocator == tmem)
	{
		DString str = dstring::temp_with_capacity(256);
		readline_to_stream(&str, stream, limit)!;
		return str.str_view();
	}
	@pool()
	{
		DString str = dstring::temp_with_capacity(256);
		readline_to_stream(&str, stream, limit)!;
		return str.copy_str(allocator);
	};
}

<*
 Reads a string, see `readline`, the data is passed to an outstream

 @param out_stream : `The stream to write to`
 @param in_stream : `The stream to read from.`
 @param limit : `Optionally limits the byte-length of the allocated output string.`
 @require @is_not_instream_if_ptr(in_stream) : "The value for 'in_stream' should have been passed as a pointer and not as a value, please add '&'."
 @require @is_not_outstream_if_ptr(out_stream) : "The value for 'out_stream' should have been passed as a pointer and not as a value, please add '&'."
 @require @is_instream(in_stream) : `The in_stream must implement InStream.`
 @require @is_outstream(out_stream) : `The out_stream must implement OutStream.`
 @return `The number of bytes written. When a 'limit' is provided and the return value is equal to it, there may be more to read on the current line.`
*>
macro usz? readline_to_stream(out_stream, in_stream = io::stdin(), usz limit = 0)
{
	return readline_to_stream_impl{$typeof(in_stream), $typeof(out_stream)}(out_stream, in_stream, limit);
}

fn usz? readline_to_stream_impl(OStream out_stream, IStream in_stream, usz limit) <IStream, OStream> @private
{
	bool $is_stream = IStream == InStream;
	$if $is_stream:
		var func @safeinfer = &in_stream.read_byte;
		char val = func((void*)in_stream)!;
	$else
		char val = in_stream.read_byte()!;
	$endif
	bool $is_out_stream = OStream == OutStream;
	$if $is_out_stream:
		var out_func @safeinfer = &out_stream.write_byte;
	$endif

	if (val == '\n') return 0;
	usz len;
	if (val != '\r')
	{
		$if $is_out_stream:
			out_func((void*)out_stream.ptr, val)!;
		$else
			out_stream.write_byte(val)!;
		$endif
		len++;
	}
	while (!limit || len < limit)
	{
		$if $is_stream:
			char? c = func((void*)in_stream);
		$else
			char? c = in_stream.read_byte();
		$endif
		if (catch err = c)
		{
			if (err == io::EOF) break;
			return err~;
		}
		if (c == '\r') continue;
		if (c == '\n') break;
		$if $is_out_stream:
			out_func((void*)out_stream.ptr, c)!;
		$else
			out_stream.write_byte(c)!;
		$endif
		len++;
	}
	return len;
}


// Additional tests for cloc
char *raw = `multiline
raw string with // and /*
markers inside`;

/* Unfinished block comment at the end of the file

these are valid in c3

