import { llmsIndex } from '@/lib/llms';

export const revalidate = false;

export async function GET() {
  return new Response(await llmsIndex(), {
    headers: { 'Content-Type': 'text/markdown; charset=utf-8' },
  });
}
