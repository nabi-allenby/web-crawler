import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { createCrawl } from "../lib/api";
import { Card, CardContent, CardHeader, CardTitle } from "../components/ui/card";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";

const schema = z.object({
  url: z.string().url("Please enter a valid URL"),
  depth: z.number().min(1).max(5),
  targeted: z.boolean(),
});

type FormData = z.infer<typeof schema>;

export default function NewCrawl() {
  const navigate = useNavigate();
  const [error, setError] = useState("");
  const [submitting, setSubmitting] = useState(false);

  const {
    register,
    handleSubmit,
    watch,
    setValue,
    formState: { errors },
  } = useForm<FormData>({
    resolver: zodResolver(schema),
    defaultValues: { url: "", depth: 2, targeted: false },
  });

  const depth = watch("depth");

  const onSubmit = async (data: FormData) => {
    setSubmitting(true);
    setError("");
    try {
      const result = await createCrawl(data.url, data.depth, data.targeted || undefined);
      navigate(`/crawls/${result.crawl_id}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to start crawl");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="max-w-2xl mx-auto space-y-6">
      <div>
        <h1 className="text-2xl font-bold text-gray-900">New Crawl</h1>
        <p className="text-gray-500 mt-1">
          Submit a URL to start crawling. The crawler will discover and index
          linked pages up to the specified depth.
        </p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Crawl Configuration</CardTitle>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit(onSubmit)} className="space-y-6">
            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1">
                URL to Crawl
              </label>
              <Input
                type="url"
                placeholder="https://example.com"
                {...register("url")}
              />
              {errors.url && (
                <p className="text-red-500 text-sm mt-1">
                  {errors.url.message}
                </p>
              )}
            </div>

            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1">
                Crawl Depth: {depth}
              </label>
              <input
                type="range"
                min={1}
                max={5}
                value={depth}
                onChange={(e) => setValue("depth", Number(e.target.value))}
                className="w-full h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer accent-blue-600"
              />
              <div className="flex justify-between text-xs text-gray-400 mt-1">
                <span>1 (shallow)</span>
                <span>5 (deep)</span>
              </div>
              {errors.depth && (
                <p className="text-red-500 text-sm mt-1">
                  {errors.depth.message}
                </p>
              )}
            </div>

            <div className="flex items-start gap-3">
              <input
                type="checkbox"
                id="targeted"
                {...register("targeted")}
                className="mt-1 h-4 w-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
              />
              <label htmlFor="targeted" className="text-sm">
                <span className="font-medium text-gray-700">
                  Targeted crawl
                </span>
                <p className="text-gray-500 mt-0.5">
                  Only follow links within the same registered domain as the
                  root URL. For example, crawling{" "}
                  <code className="text-xs bg-gray-100 px-1 rounded">
                    blog.example.com
                  </code>{" "}
                  will also crawl{" "}
                  <code className="text-xs bg-gray-100 px-1 rounded">
                    shop.example.com
                  </code>{" "}
                  but not external sites.
                </p>
              </label>
            </div>

            <div className="bg-blue-50 border border-blue-200 rounded-lg p-4">
              <h4 className="text-sm font-medium text-blue-800 mb-1">
                What to expect
              </h4>
              <p className="text-sm text-blue-700">
                Depth {depth} will crawl the target URL and follow links up to{" "}
                {depth} level{depth > 1 ? "s" : ""} deep. Higher depth values
                discover more pages but take longer to complete.
              </p>
            </div>

            {error && (
              <div className="bg-red-50 border border-red-200 rounded-lg p-4">
                <p className="text-sm text-red-700">{error}</p>
              </div>
            )}

            <div className="flex gap-3">
              <Button type="submit" disabled={submitting}>
                {submitting ? "Starting Crawl..." : "Start Crawl"}
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={() => navigate("/")}
              >
                Cancel
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
